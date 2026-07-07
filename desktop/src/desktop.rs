use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use log::{error, info};
use massive_util::CollectingVec;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use massive_applications::{
    CreationMode, InstanceEnvironment, InstanceEvent, InstanceId, InstanceParameters,
    InstanceSubmission, ViewEvent,
};
use massive_input::EventManager;
use massive_renderer::RenderPacing;
use massive_scene::ChangeCollector;
use massive_shell::AsyncWindowRenderer;
use massive_shell::{ApplicationContext, FontManager, Scene, ShellEvent};
use uuid::Uuid;

use crate::DesktopEnvironment;
use crate::desktop_system::change::{Changes, DesktopChange};
use crate::desktop_system::{
    Commands, DesktopCommand, DesktopSystem, ProjectCommand, TransactionEffectsMode,
};
use crate::instance_manager::InstanceManager;
use crate::instance_presenter::InstanceRoot;
use crate::projects::{
    LaunchProfile, LaunchProfileId, Launcher, LauncherMode, MatrixPlacement, Project,
    ProjectConfiguration, ProjectId, ProjectProperties, ProjectSet,
};

#[derive(Debug)]
pub struct Desktop {
    scene: Scene,
    renderer: AsyncWindowRenderer,
    system: DesktopSystem,

    event_manager: EventManager<ViewEvent>,

    instance_manager: InstanceManager,
    instance_submissions: UnboundedReceiver<(InstanceId, InstanceSubmission)>,
    context: ApplicationContext,
}

impl Desktop {
    pub async fn new(env: DesktopEnvironment, context: ApplicationContext) -> Result<Self> {
        // Load configuration

        let projects_dir = env.projects_dir();
        let project_configuration = ProjectConfiguration::from_dir(projects_dir.as_deref())?;
        let project_set = ProjectSet::from_configuration(project_configuration)?;

        // Create the font manager - shared between desktop and instances
        let fonts = FontManager::system();

        // Create scene early for presenter initialization
        let scene_changes = Arc::new(ChangeCollector::default());
        let scene = context.new_scene_with_change_collector(scene_changes.clone());

        let (submissions_tx, mut submissions_rx) = unbounded_channel();
        let environment = InstanceEnvironment::new(
            submissions_tx,
            context.primary_monitor_scale_factor(),
            fonts.clone(),
        );

        let mut instance_manager = InstanceManager::new(environment);
        // We need to use ViewEvent early on, because the `EventRouter` isn't able to convert events.
        let event_manager = EventManager::<ViewEvent>::default();

        // Start one instance of the first registered application
        let primary_application = env
            .applications
            .get_named(&env.primary_application)
            .expect("No primary application");

        let primary_root = InstanceRoot::new(&scene);
        let primary_instance = Uuid::new_v4().into();
        instance_manager.spawn(
            primary_instance,
            primary_application,
            CreationMode::New(InstanceParameters::new()),
            primary_root.location(),
        )?;

        // First wait for the initial submission so the window can match the primary view.
        let Some((initial_instance, initial_submission)) = submissions_rx.recv().await else {
            bail!("Did not receive the initial submission from the application");
        };

        let primary_instance = initial_instance;
        let creation_info = initial_submission
            .primary_view_creation_info()?
            .context("Initial submission did not create a primary view")?;

        // Currently we can't target views directly, the focus system is targeting only instances
        // and their primary view.
        let default_size = creation_info.size();

        let window = context.new_window(creation_info.size()).await?;
        let renderer = window
            .renderer()
            .with_shapes()
            .with_text(fonts.clone())
            .with_background_color(massive_geometry::Color::BLACK)
            .build()
            .await?;

        // Initial setup

        let primary_project = primary_project();

        // Architecture: Providing the root group here is conceptually wrong I guess, because it
        // does not exist yet.
        let mut system =
            DesktopSystem::new(env, fonts.clone(), window.clone(), default_size, &scene)?;

        let primary_project_commands = primary_project.commands.map(DesktopCommand::Project);

        let project_setup_commands: Commands =
            project_set_to_commands(&project_set).map(DesktopCommand::Project);

        let primary_instance_commands: Commands = [DesktopCommand::StartInstance {
            launcher: primary_project.primary_launcher,
            instance: primary_instance,
            root: Some(primary_root),
            parameters: InstanceParameters::new(),
        }]
        .into();

        let initial_submission_changes: Changes =
            DesktopChange::IntegrateInstanceSubmission(primary_instance, initial_submission).into();

        let commands =
            primary_project_commands + project_setup_commands + primary_instance_commands;

        let mut changes = Changes::Empty;
        for command in commands {
            changes += system.plan(command, &scene)?;
        }

        system.transact(
            changes + initial_submission_changes,
            &scene,
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;

        let desktop = Self {
            scene,
            renderer,
            system,
            event_manager,
            instance_manager,
            instance_submissions: submissions_rx,
            context,
        };
        Ok(desktop)
    }

    pub async fn run(&mut self) -> Result<()> {
        loop {
            tokio::select! {
                Some((instance_id, submission)) = self.instance_submissions.recv() => {
                    self.integrate_instance_submission(instance_id, submission)?;
                }

                shell_event = self.context.wait_for_shell_event() => {
                    let event = shell_event?;

                    match event {
                        ShellEvent::WindowEvent(_window_id, window_event) => {
                            if let Some(view_event) = ViewEvent::from_window_event(&window_event)
                                && let Some(input_event) =
                                    self.event_manager.add_event(view_event, Instant::now())
                            {
                                self.system.update_pointer_feedback(&input_event);

                                let keyboard_shortcut = self.system.match_desktop_keyboard_shortcut(&input_event);

                                let changes : Changes = if let Some(keyboard_cmd) = keyboard_shortcut {
                                        self.system.plan(keyboard_cmd.into_command(), &self.scene)?
                                    } else {
                                        self.system.process_input_event(
                                            &input_event,
                                            self.renderer.geometry(),
                                        )?
                                    };

                                self.system.transact(
                                    changes,
                                    &self.scene,
                                    &mut self.instance_manager,
                                    None,
                                )?;
                            }

                            self.renderer.resize_redraw(&window_event)?;
                        }
                        ShellEvent::ApplyAnimations(_) => {
                            // Performance: Not every instance needs that, only the ones animating.
                            self.instance_manager.broadcast_event(InstanceEvent::ApplyAnimations);
                            self.system.apply_animations();
                        }
                    }
                }

                Ok((instance_id, instance_result)) = self.instance_manager.join_next() => {
                    info!("Instance ended (submissions pending: {}): {instance_id:?}", self.instance_submissions.len());

                    if self.system.is_present(&instance_id) {
                        // Did it end on its own? -> Act as if the user ended it.
                        // Robustness: This should probably handled differently.
                        let changes = self.system.plan(DesktopCommand::StopInstance(instance_id), &self.scene)?;
                        self.system.transact(
                            changes,
                            &self.scene,
                            &mut self.instance_manager,
                            None,
                        )?;
                    }

                    // Feature: Display the error to the user?

                    if let Err(e) = instance_result {
                        log::warn!("Instance returned error: {e}");
                    }

                    // If all instances have finished, exit
                    if self.instance_manager.is_empty() {
                        let queued_submissions = self.instance_submissions.len();
                        if queued_submissions > 0 {
                            error!(
                                "Desktop exiting with queued instance submissions after all instances finished: queued_submissions={queued_submissions}"
                            );
                        }
                        return Ok(());
                    }
                }

                else => {
                    return Ok(());
                }
            }

            // Get the camera, build the frame, and submit it to the renderer.
            {
                let camera = *self.system.camera();
                let mut frame = self.scene.begin_frame().with_camera(camera);
                if self.system.effective_pacing() == RenderPacing::Smooth {
                    frame = frame.with_pacing(RenderPacing::Smooth);
                }
                frame.submit_to(&mut self.renderer)?;
            }
        }
    }

    fn integrate_instance_submission(
        &mut self,
        instance: InstanceId,
        submission: InstanceSubmission,
    ) -> Result<()> {
        self.system.transact(
            DesktopChange::IntegrateInstanceSubmission(instance, submission),
            &self.scene,
            &mut self.instance_manager,
            None,
        )
    }
}

#[derive(Debug)]
struct PrimaryProject {
    primary_launcher: LaunchProfileId,
    commands: CollectingVec<ProjectCommand>,
}

fn primary_project() -> PrimaryProject {
    let mut commands = CollectingVec::default();

    let primary_project = ProjectId::new();
    let primary_launcher = LaunchProfileId::new();

    commands += ProjectCommand::AddProject {
        id: primary_project,
        properties: ProjectProperties {
            name: "Primary / Local".into(),
        },
    };

    commands += ProjectCommand::AddLauncher {
        project: primary_project,
        id: primary_launcher,
        profile: LaunchProfile {
            name: "Primary / Local".into(),
            mode: LauncherMode::Band,
            tags: Vec::new(),
            params: Default::default(),
        },
        placement: MatrixPlacement { column: 0, row: 0 },
    };

    PrimaryProject {
        primary_launcher,
        commands,
    }
}

fn project_set_to_commands(project_set: &ProjectSet) -> CollectingVec<ProjectCommand> {
    let mut commands = CollectingVec::Empty;

    commands.push(ProjectCommand::SetStartupProfile(project_set.start));

    for project in &project_set.projects {
        project_commands(project, &mut commands);
    }

    commands
}

fn project_commands(project: &Project, commands: &mut CollectingVec<ProjectCommand>) {
    commands.push(ProjectCommand::AddProject {
        id: project.id,
        properties: project.properties.clone(),
    });

    for launcher in &project.launchers {
        launcher_commands(project.id, launcher, commands);
    }
}

fn launcher_commands(
    project: ProjectId,
    launcher: &Launcher,
    commands: &mut CollectingVec<ProjectCommand>,
) {
    commands.push(ProjectCommand::AddLauncher {
        project,
        id: launcher.id,
        profile: launcher.profile.clone(),
        placement: launcher.placement,
    })
}
