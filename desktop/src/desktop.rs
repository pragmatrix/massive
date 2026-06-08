use std::{sync::Arc, time::Instant};

use anyhow::{Context, Result, bail};
use log::{error, info};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use massive_applications::{
    CreationMode, InstanceChange, InstanceEnvironment, InstanceEvent, InstanceId,
    InstanceParameters, InstanceSubmission, ViewCreationInfo, ViewEvent, ViewRole,
};
use massive_input::EventManager;
use massive_renderer::RenderPacing;
use massive_scene::ChangeCollector;
use massive_shell::{ApplicationContext, FontManager, Scene, ShellEvent};
use massive_shell::{AsyncWindowRenderer, ShellWindow};

use crate::DesktopEnvironment;
use crate::desktop_system::{
    DesktopCommand, DesktopSystem, ProjectCommand, TransactionEffectsMode,
};
use crate::event_sourcing::Transaction;
use crate::instance_manager::{InstanceManager, InstanceRoot, ViewPath};
use crate::projects::{
    LaunchProfile, LaunchProfileId, Launcher, LauncherMode, MatrixPlacement, Project,
    ProjectConfiguration, ProjectId, ProjectProperties, ProjectSet,
};

#[derive(Debug)]
pub struct Desktop {
    scene: Scene,
    renderer: AsyncWindowRenderer,
    window: ShellWindow,
    cursor_visible: bool,
    system: DesktopSystem,

    event_manager: EventManager<ViewEvent>,

    instance_manager: InstanceManager,
    // May need to move this into the ApplicationContext.
    scene_changes: Arc<ChangeCollector>,
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

        instance_manager.spawn(
            primary_application,
            CreationMode::New(InstanceParameters::new()),
            InstanceRoot::new(&scene),
        )?;

        // First wait for and interpret the initial submission.
        let Some((initial_instance, initial_submission)) = submissions_rx.recv().await else {
            bail!("Did not receive the initial submission from the application");
        };

        let initial_interpretation = Self::interpret_instance_submission(
            initial_instance,
            initial_submission,
            &scene_changes,
        )?;

        let primary_instance = initial_instance;
        let creation_info = initial_interpretation
            .primary_view_creation_info
            .clone()
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

        let primary_project_transaction = primary_project.transaction.map(DesktopCommand::Project);

        let project_setup_transaction =
            project_set_to_transaction(&project_set).map(DesktopCommand::Project);

        let primary_instance_transaction: Transaction<_> = [DesktopCommand::PresentInstance {
            launcher: primary_project.primary_launcher,
            instance: primary_instance,
        }]
        .into();

        let initial_submission_transaction: Transaction<_> =
            initial_interpretation.pending_commands.into();

        system.transact(
            primary_project_transaction
                + project_setup_transaction
                + primary_instance_transaction
                + initial_submission_transaction,
            &scene,
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;

        system.set_instance_pacing(primary_instance, initial_interpretation.pacing);

        let desktop = Self {
            scene,
            renderer,
            window,
            cursor_visible: true,
            system,
            event_manager,
            instance_manager,
            scene_changes,
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
                                let (cmd, effects) = self.system.process_input_event(
                                    &input_event,
                                    &self.instance_manager,
                                    self.renderer.geometry(),
                                )?;
                                let cursor_visible = self.system.is_cursor_visible();
                                if self.cursor_visible != cursor_visible {
                                    self.window.set_cursor_visible(cursor_visible);
                                    self.cursor_visible = cursor_visible;
                                }
                                self.system.transact_with_effects(
                                    cmd,
                                    &self.scene,
                                    &mut self.instance_manager,
                                    if input_event.any_buttons_pressed() {
                                        TransactionEffectsMode::CameraLocked
                                    } else {
                                        TransactionEffectsMode::Normal
                                    },
                                    effects,
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
                        // Did it end on its own? -> Act as such that the user ended it.
                        // Robustness: This should probably handled differently.
                        self.system.transact(
                            DesktopCommand::StopInstance(instance_id),
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
        let interpretation =
            Self::interpret_instance_submission(instance, submission, &self.scene_changes)?;

        if !interpretation.pending_commands.is_empty() {
            let effects_mode = if self.system.any_buttons_pressed() {
                TransactionEffectsMode::CameraLocked
            } else {
                TransactionEffectsMode::Normal
            };

            self.system.transact(
                interpretation.pending_commands,
                &self.scene,
                &mut self.instance_manager,
                effects_mode,
            )?;
        }

        self.system
            .set_instance_pacing(instance, interpretation.pacing);
        Ok(())
    }

    fn interpret_instance_submission(
        instance: InstanceId,
        submission: InstanceSubmission,
        scene_changes: &ChangeCollector,
    ) -> Result<SubmissionInterpretation> {
        let (changes, pacing) = submission.into_parts();
        let mut interpretation = SubmissionInterpretation {
            pacing,
            pending_commands: Vec::new(),
            primary_view_creation_info: None,
        };

        for change in changes.release() {
            Self::process_instance_change(instance, change, scene_changes, &mut interpretation)?;
        }
        Ok(interpretation)
    }

    fn process_instance_change(
        instance: InstanceId,
        change: InstanceChange,
        scene_changes: &ChangeCollector,
        interpretation: &mut SubmissionInterpretation,
    ) -> Result<()> {
        match change {
            InstanceChange::Scene(change) => {
                scene_changes.collect(change);
            }
            InstanceChange::CreateView(info) => {
                match info.role {
                    ViewRole::Primary => {
                        if interpretation
                            .primary_view_creation_info
                            .replace(info.clone())
                            .is_some()
                        {
                            bail!("Submission created multiple primary views");
                        }
                    }
                    ViewRole::Assistant | ViewRole::Notification { .. } => {}
                }

                interpretation
                    .pending_commands
                    .push(DesktopCommand::PresentView(instance, info));
            }
            InstanceChange::DestroyView(id) => {
                let view_path: ViewPath = (instance, id).into();
                interpretation
                    .pending_commands
                    .push(DesktopCommand::HideView(view_path));
            }
            InstanceChange::View(view_id, command) => {
                let view_path: ViewPath = (instance, view_id).into();
                interpretation
                    .pending_commands
                    .push(DesktopCommand::ApplyViewChange(view_path, command));
            }
            // This makes sure that all pending Scene Changes from the Instance have been collected
            // before we drop the last ref the instance has to its parent location.
            InstanceChange::End(_) => {}
        }
        Ok(())
    }
}

struct SubmissionInterpretation {
    pacing: RenderPacing,
    pending_commands: Vec<DesktopCommand>,
    primary_view_creation_info: Option<ViewCreationInfo>,
}

#[derive(Debug)]
struct PrimaryProject {
    primary_launcher: LaunchProfileId,
    transaction: Transaction<ProjectCommand>,
}

fn primary_project() -> PrimaryProject {
    let mut cmds = Vec::new();

    let primary_project = ProjectId::new();
    let primary_launcher = LaunchProfileId::new();

    cmds.push(ProjectCommand::AddProject {
        id: primary_project,
        properties: ProjectProperties {
            name: "Primary / Local".into(),
        },
    });
    cmds.push(ProjectCommand::AddLauncher {
        project: primary_project,
        id: primary_launcher,
        profile: LaunchProfile {
            name: "Primary / Local".into(),
            mode: LauncherMode::Band,
            tags: Vec::new(),
            params: Default::default(),
        },
        placement: MatrixPlacement { column: 0, row: 0 },
    });

    PrimaryProject {
        primary_launcher,
        transaction: cmds.into(),
    }
}

fn project_set_to_transaction(project_set: &ProjectSet) -> Transaction<ProjectCommand> {
    let mut commands = Vec::new();

    commands.push(ProjectCommand::SetStartupProfile(project_set.start));

    for project in &project_set.projects {
        project_commands(project, &mut commands);
    }

    commands.into()
}

fn project_commands(project: &Project, commands: &mut Vec<ProjectCommand>) {
    commands.push(ProjectCommand::AddProject {
        id: project.id,
        properties: project.properties.clone(),
    });

    for launcher in &project.launchers {
        launcher_commands(project.id, launcher, commands);
    }
}

fn launcher_commands(project: ProjectId, launcher: &Launcher, commands: &mut Vec<ProjectCommand>) {
    commands.push(ProjectCommand::AddLauncher {
        project,
        id: launcher.id,
        profile: launcher.profile.clone(),
        placement: launcher.placement,
    })
}
