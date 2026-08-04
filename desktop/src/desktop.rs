use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use log::{error, info};
use massive_util::CollectingVec;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use massive_applications::{
    ApplicationEvent, ApplicationMessage, CreationMode, Frame, InstanceEnvironment, InstanceId,
    InstanceParameters, InstanceSubmission, ViewEvent,
};
use massive_input::EventManager;
use massive_renderer::RenderPacing;
use massive_scene::ChangeCollector;
use massive_shell::AsyncWindowRenderer;
use massive_shell::{ApplicationContext, FontManager, Scene};
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

#[derive(Debug)]
enum DesktopEvent {
    ApplicationEvents(Vec<ApplicationEvent<Infallible>>),
    InstanceSubmission(InstanceId, InstanceSubmission),
    InstanceEnded(InstanceId, massive_shell::Result<()>),
}

impl Desktop {
    pub async fn new(env: DesktopEnvironment, mut context: ApplicationContext) -> Result<Self> {
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
        let mut renderer = window
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
        let mut system = DesktopSystem::new(
            env,
            fonts.clone(),
            window,
            default_size,
            &scene,
            context.movement_runtime(),
        )?;

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

        let mut frame = context.frame(&scene);
        system.transact(
            changes + initial_submission_changes,
            &mut frame,
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;
        submit_frame(&mut system, frame, &mut renderer)?;

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
            let event = tokio::select! {
                Some((instance_id, submission)) = self.instance_submissions.recv() => {
                    DesktopEvent::InstanceSubmission(instance_id, submission)
                }

                events = self.context.wait_for_events::<Infallible>() => {
                    DesktopEvent::ApplicationEvents(events?)
                }

                instance = self.instance_manager.join_next() => {
                    let (instance_id, instance_result) = instance?;
                    DesktopEvent::InstanceEnded(instance_id, instance_result)
                }
            };

            let mut frame = self.context.frame(&self.scene);

            match event {
                DesktopEvent::ApplicationEvents(events) => {
                    for event in events {
                        match event {
                            ApplicationEvent::View(_, view_event) => {
                                if let Some(input_event) = self
                                    .event_manager
                                    .add_event(view_event.clone(), Instant::now())
                                {
                                    let keyboard_shortcut =
                                        self.system.match_desktop_keyboard_shortcut(&input_event);

                                    let event_changes: Changes =
                                        if let Some(keyboard_cmd) = keyboard_shortcut {
                                            self.system
                                                .plan(keyboard_cmd.into_command(), &self.scene)?
                                        } else {
                                            self.system.process_input_event(
                                                &input_event,
                                                self.renderer.geometry(),
                                            )?
                                        };

                                    self.system.transact(
                                        event_changes,
                                        &mut frame,
                                        &mut self.instance_manager,
                                        None,
                                    )?;
                                }

                                // This is completely weird here. We need a better solution for resize_redraw().
                                self.renderer.resize_redraw(&view_event)?;
                            }
                            ApplicationEvent::ApplyAnimations(presentation_id) => {
                                frame.upgrade_to_apply_animations_cycle();
                                let animating_instances =
                                    self.system.animating_instances().collect::<Vec<_>>();
                                for instance in animating_instances {
                                    _ = self.instance_manager.send_event(
                                        instance,
                                        ApplicationMessage::ApplyAnimations(presentation_id),
                                    );
                                }
                            }
                            ApplicationEvent::Shutdown(_) => {
                                // Robustness: Clarify if and when this happens.
                                info!("Desktop shutdown request received");
                                return Ok(());
                            }
                            ApplicationEvent::Custom(event) => match event {},
                        }
                    }
                }
                DesktopEvent::InstanceSubmission(instance, submission) => self.system.transact(
                    DesktopChange::IntegrateInstanceSubmission(instance, submission),
                    &mut frame,
                    &mut self.instance_manager,
                    None,
                )?,
                DesktopEvent::InstanceEnded(instance_id, instance_result) => {
                    info!(
                        "Instance ended (submissions pending: {}): {instance_id:?}",
                        self.instance_submissions.len()
                    );

                    if self.system.is_present(&instance_id) {
                        // Did it end on its own? -> Act as if the user ended it.
                        // Robustness: This should probably handled differently.
                        let changes = self
                            .system
                            .plan(DesktopCommand::StopInstance(instance_id), &self.scene)?;
                        self.system.transact(
                            changes,
                            &mut frame,
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
            }

            submit_frame(&mut self.system, frame, &mut self.renderer)?;
        }
    }
}

fn submit_frame(
    system: &mut DesktopSystem,
    frame: Frame,
    renderer: &mut AsyncWindowRenderer,
) -> Result<()> {
    let camera = *system.camera(frame.animation_time());
    let mut submission = frame.submission().render_submission().with_camera(camera);
    // If any instance runs on smooth pacing, we need to, too.
    if system.effective_pacing() == RenderPacing::Smooth {
        submission = submission.with_pacing(RenderPacing::Smooth);
    }
    submission.submit_to(renderer)
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
        after: None,
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
        after: None,
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
