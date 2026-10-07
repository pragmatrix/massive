use std::convert::Infallible;
use std::fs;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use derive_more::Constructor;
use log::{error, info};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::time::{Duration, Instant as TokioInstant, sleep_until};
use uuid::Uuid;

use massive_applications::{
    ApplicationEvent, ApplicationMessage, CreationMode, Frame, InstanceEnvironment, InstanceId,
    InstanceParameters, InstanceSubmission, ViewEvent, begin_frame,
};
use massive_input::EventManager;
use massive_renderer::RenderPacing;
use massive_shell::{ApplicationContext, AsyncWindowRenderer, ShellWindow};

use crate::DesktopEnvironment;
use crate::desktop_system::change::{Changes, DesktopChange, DesktopSystemEffect};
use crate::desktop_system::{Commands, DesktopCommand, DesktopSystem, TransactionEffectsMode};
use crate::instance_manager::InstanceManager;
use crate::instance_presenter::{InstanceKind, InstanceRoot};
use crate::projects::persistence::{self, ConfigurationPersistence};
use crate::projects::{RuntimeConfiguration, to_commands};
use crate::window_state::WindowPresentationState;
use crate::window_state::WindowState;

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct Desktop {
    window: ShellWindow,
    window_presentation_state: WindowPresentationState,

    renderer: AsyncWindowRenderer,
    system: DesktopSystem,
    /// Persists configuration changes after a transaction (ADR 0013).
    /// Setup replays the parsed configuration without persisting it.
    configuration_persistence: ConfigurationPersistence,

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
    pub async fn new(env: DesktopEnvironment, context: ApplicationContext) -> Result<Self> {
        // Load configuration

        let (configuration_persistence, configuration) = load_configuration(&env)?;

        // The desktop task's change queue: installed by the shell's application task context
        // (ADR 0008). Presenters submit their handles through the ambient accessors.

        let (submissions_tx, mut submissions_rx) = unbounded_channel();
        let environment =
            InstanceEnvironment::new(submissions_tx, context.primary_monitor_scale_factor());
        let mut instance_manager = InstanceManager::new(environment);

        // We need to use ViewEvent early on, because the `EventRouter` isn't able to convert events.
        let event_manager = EventManager::<ViewEvent>::default();

        // Start one instance of the first registered application
        let primary_application = env
            .applications
            .get_named(&env.primary_application)
            .expect("No primary application");

        let primary_root = InstanceRoot::new();
        let primary_instance = Uuid::new_v4().into();
        instance_manager.spawn(
            primary_instance,
            primary_application,
            CreationMode::New(InstanceParameters::new()),
            primary_root.view_parent(),
        )?;

        // First wait for the initial submission so the window can match the primary view.
        let Some((initial_instance, initial_submission)) = submissions_rx.recv().await else {
            bail!("Did not receive the initial submission from the application");
        };

        let primary_instance = initial_instance;
        let creation_info = match initial_submission.primary_view_creation_info()? {
            Some(info) => info,
            None => {
                // The instance ended before creating a primary view; surface its error.
                let (_, result) = instance_manager.join_next().await?;
                result.context("Instance failed during startup")?;
                bail!("Initial submission did not create a primary view");
            }
        };

        // Currently we can't target views directly, the focus system is targeting only instances
        // and their primary view.
        let default_size = creation_info.size();

        let window = context.new_window(creation_info.size()).await?;
        let mut renderer = window
            .renderer()
            .with_shapes()
            .with_text()
            .with_background_color(massive_geometry::Color::BLACK)
            .build()
            .await?;

        // Initial setup

        // The boot commands derive from the aggregate while it is still owned
        // here; `DesktopSystem::new` then takes it over, so the live model's
        // placements come from this same instance the commands were read from.
        let project_setup_commands: Commands =
            to_commands(&configuration).map(DesktopCommand::Project);
        let boot_launcher = configuration
            .boot_launcher()
            .expect("the configuration load gives a launcher-less configuration one");

        let mut system = DesktopSystem::new(env, default_size, configuration)?;

        // The session boots into the startup launcher the configuration names. The
        // load gives a launcher-less configuration one, and the fallback derived
        // here is the first launcher of the loaded aggregate before any command
        // replays.
        let primary_instance_commands: Commands = [DesktopCommand::StartInstance {
            launcher: boot_launcher,
            instance: primary_instance,
            root: Some(primary_root),
            parameters: InstanceParameters::new(),
            kind: InstanceKind::Base,
        }]
        .into();

        let initial_submission_changes: Changes =
            DesktopChange::IntegrateInstanceSubmission(primary_instance, initial_submission).into();

        let commands = project_setup_commands + primary_instance_commands;

        let mut changes = Changes::Empty;
        for command in commands {
            changes += system.plan(command)?;
        }
        let frame = begin_frame();
        system.transact(
            changes + initial_submission_changes,
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;
        let mut presentation_state = WindowPresentationState::default();
        finalize_desktop_frame(
            &mut system,
            frame,
            &window,
            &mut presentation_state,
            &mut renderer,
        )?;

        let desktop = Self {
            window,
            window_presentation_state: presentation_state,
            renderer,
            system,
            configuration_persistence,
            event_manager,
            instance_manager,
            instance_submissions: submissions_rx,
            context,
        };
        Ok(desktop)
    }

    pub async fn run(&mut self) -> Result<()> {
        // A close request hands active rendering over to the bounded drain below.
        self.run_active().await?;
        self.run_shutdown().await
    }

    async fn run_active(&mut self) -> Result<()> {
        while !self.instance_manager.is_empty() {
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

            let mut frame = begin_frame();

            match event {
                DesktopEvent::ApplicationEvents(events) => {
                    for event in events {
                        match event {
                            ApplicationEvent::FullscreenRequested => {
                                let changes = self.system.plan(DesktopCommand::ToggleFullScreen)?;
                                self.transact_and_persist(changes)?;
                            }
                            ApplicationEvent::View(_, ViewEvent::CloseRequested) => {
                                return Ok(());
                            }
                            ApplicationEvent::View(_, view_event) => {
                                let mut desktop_changes = Changes::default();

                                if let ViewEvent::Resized(size_px) = &view_event {
                                    // For some reason this does not match.
                                    // `debug_assert_eq!(self.window.inner_size(), *size_px);`
                                    // The system ignores resize events that match its current state.
                                    desktop_changes <<= DesktopChange::WindowResized(
                                        WindowState::new(*size_px, self.window.is_fullscreen()),
                                    );
                                }

                                if let Some(input_event) = self
                                    .event_manager
                                    .add_event(view_event.clone(), Instant::now())
                                {
                                    let keyboard_shortcut =
                                        self.system.match_desktop_keyboard_shortcut(&input_event);

                                    let input_changes: Changes =
                                        if let Some(keyboard_cmd) = keyboard_shortcut {
                                            self.system.plan(keyboard_cmd)?
                                        } else {
                                            self.system.process_input_event(
                                                &input_event,
                                                self.renderer.geometry(),
                                            )?
                                        };

                                    desktop_changes += input_changes;
                                }

                                self.transact_and_persist(desktop_changes)?;

                                // This is completely weird here. We need a better solution for resize_redraw().
                                self.renderer.resize_redraw(&view_event)?;
                            }
                            ApplicationEvent::ApplyAnimations(tick) => {
                                frame.upgrade_to_apply_animations_cycle(tick.vblank_time);
                                let animating_instances =
                                    self.system.animating_instances().collect::<Vec<_>>();
                                for instance in animating_instances {
                                    _ = self.instance_manager.send_event(
                                        instance,
                                        ApplicationMessage::ApplyAnimations(tick),
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
                DesktopEvent::InstanceSubmission(instance, submission) => {
                    self.transact_and_persist(DesktopChange::IntegrateInstanceSubmission(
                        instance, submission,
                    ))?;
                }
                DesktopEvent::InstanceEnded(instance_id, instance_result) => {
                    self.handle_instance_ended((instance_id, instance_result))?;
                }
            }

            finalize_desktop_frame(
                &mut self.system,
                frame,
                &self.window,
                &mut self.window_presentation_state,
                &mut self.renderer,
            )?;
        }
        Ok(())
    }

    async fn run_shutdown(&mut self) -> Result<()> {
        let instance_count = self.instance_manager.len();
        if instance_count == 0 {
            return Ok(());
        }

        self.instance_manager.request_shutdown_all()?;
        let shutdown_deadline = TokioInstant::now() + SHUTDOWN_TIMEOUT;
        info!("Graceful shutdown started for {instance_count} instances");

        while !self.instance_manager.is_empty() {
            let event = tokio::select! {
                _ = sleep_until(shutdown_deadline) => {
                    let unfinished = self.instance_manager.instance_ids().collect::<Vec<_>>();
                    error!("Shutdown deadline expired: unfinished instances {unfinished:?}");
                    bail!("Shutdown deadline expired");
                }
                Some((instance_id, submission)) = self.instance_submissions.recv() => {
                    DesktopEvent::InstanceSubmission(instance_id, submission)
                }
                instance = self.instance_manager.join_next() => {
                    let (instance_id, instance_result) = instance?;
                    DesktopEvent::InstanceEnded(instance_id, instance_result)
                }
            };

            let frame = begin_frame();
            match event {
                DesktopEvent::InstanceSubmission(instance, submission) => {
                    self.transact_and_persist(DesktopChange::IntegrateInstanceSubmission(
                        instance, submission,
                    ))?;
                }
                DesktopEvent::InstanceEnded(instance_id, instance_result) => {
                    self.handle_instance_ended((instance_id, instance_result))?;
                }
                DesktopEvent::ApplicationEvents(_) => unreachable!(),
            }

            finalize_desktop_frame(
                &mut self.system,
                frame,
                &self.window,
                &mut self.window_presentation_state,
                &mut self.renderer,
            )?;
        }
        Ok(())
    }

    /// Integrates an instance's end: when it is still presented, the end acts
    /// as if the user stopped it; its pending submissions drain in this frame
    /// before the desktop may consider itself finished.
    fn handle_instance_ended(
        &mut self,
        (instance_id, instance_result): (InstanceId, massive_shell::Result<()>),
    ) -> Result<()> {
        info!(
            "Instance ended (submissions pending: {}): {instance_id:?}",
            self.instance_submissions.len()
        );

        if self.system.is_present(&instance_id) {
            // Did it end on its own? -> Act as if the user ended it.
            // Robustness: This should probably handled differently.
            let changes = self
                .system
                .plan(DesktopCommand::StopInstance(instance_id))?;
            self.transact_and_persist(changes)?;
        }

        // Feature: Display the error to the user?
        if let Err(e) = instance_result {
            log::warn!("Instance returned error: {e}");
        }

        // Drain final submissions into this frame before deciding that the desktop is finished.
        while let Ok((instance, submission)) = self.instance_submissions.try_recv() {
            self.transact_and_persist(DesktopChange::IntegrateInstanceSubmission(
                instance, submission,
            ))?;
        }

        Ok(())
    }

    /// Transacts the changes, and persists the aggregate when they applied a
    /// configuration change (ADR 0013).
    fn transact_and_persist(&mut self, changes: impl Into<Changes>) -> Result<()> {
        let output = self
            .system
            .transact(changes, &mut self.instance_manager, None)?;
        for effect in output.effects {
            match effect {
                DesktopSystemEffect::ToggleWindowFullScreen => self.window.toggle_fullscreen()?,
                DesktopSystemEffect::PersistConfiguration => self
                    .configuration_persistence
                    .persist(self.system.configuration()),
            }
        }
        Ok(())
    }
}

/// Loads the desktop configuration from the projects directory.
///
/// A missing configuration file means "start fresh": the built-in default
/// configuration is written first, so every later change has a file to be persisted
/// to. Any other file error fails and aborts desktop startup.
fn load_configuration(
    env: &DesktopEnvironment,
) -> Result<(ConfigurationPersistence, RuntimeConfiguration)> {
    let projects_dir = env
        .projects_dir()
        .with_context(|| "Could not resolve the projects directory (no home directory?)")?;
    let configuration_path = projects_dir.join(persistence::CONFIG_FILE_NAME);
    if !configuration_path.exists() {
        log::info!(
            "No configuration at {}, writing the default configuration",
            configuration_path.display()
        );
        persistence::write_default_config(&configuration_path)?;
    }
    let text = fs::read_to_string(&configuration_path)
        .with_context(|| format!("reading {}", configuration_path.display()))?;
    let configuration = persistence::parse_configuration(&configuration_path, &text)?;
    Ok((
        ConfigurationPersistence::new(&configuration_path),
        configuration,
    ))
}

/// Push everything out.
///
/// Update the camera, pacing, submit the frame, and update the window presentation.
fn finalize_desktop_frame(
    system: &mut DesktopSystem,
    frame: Frame,
    window: &ShellWindow,
    presentation_state: &mut WindowPresentationState,
    renderer: &mut AsyncWindowRenderer,
) -> Result<()> {
    let window_context = WindowContext::new(window, presentation_state, renderer);

    let camera = *system.camera();

    let mut submission = frame.render_submission().with_camera(camera);
    // If any instance runs on smooth pacing, we need to, too.
    if system.effective_pacing() == RenderPacing::Smooth {
        submission = submission.with_pacing(RenderPacing::Smooth);
    }
    submission.submit_to(window_context.renderer)?;

    window_context
        .presentation_state
        .delta_sync(system.window_presentation_state()?, window_context.window);
    Ok(())
}

#[derive(Debug, Constructor)]
struct WindowContext<'a> {
    window: &'a ShellWindow,
    presentation_state: &'a mut WindowPresentationState,
    renderer: &'a mut AsyncWindowRenderer,
}
