use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use log::info;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use uuid::Uuid;

use massive_applications::{
    CreationMode, InstanceCommand, InstanceEnvironment, InstanceEvent, InstanceId,
    InstanceParameters, ViewCommand, ViewEvent, ViewId, ViewRole,
};
use massive_input::{EventManager, ExternalEvent};
use massive_renderer::RenderPacing;
use massive_shell::{ApplicationContext, FontManager, Scene, ShellEvent};
use massive_shell::{AsyncWindowRenderer, ShellWindow};

use crate::DesktopEnvironment;
use crate::desktop_presenter::DesktopTarget;
use crate::desktop_system::DesktopSystem;
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::projects::{Project, ProjectConfiguration};

#[derive(Debug)]
pub struct Desktop {
    scene: Scene,
    renderer: AsyncWindowRenderer,
    window: ShellWindow,
    system: DesktopSystem,

    event_manager: EventManager<ViewEvent>,

    instance_manager: InstanceManager,
    instance_commands: UnboundedReceiver<(InstanceId, InstanceCommand)>,
    context: ApplicationContext,
    fonts: FontManager,
}

impl Desktop {
    pub async fn new(env: DesktopEnvironment, context: ApplicationContext) -> Result<Self> {
        // Load configuration

        let projects_dir = env.projects_dir();
        let project_configuration = ProjectConfiguration::from_dir(projects_dir.as_deref())?;
        let project = Project::from_configuration(project_configuration)?;

        // Create the font manager - shared between desktop and instances
        let fonts = FontManager::system();

        // Create scene early for presenter initialization
        let scene = context.new_scene();

        let (requests_tx, mut requests_rx) = unbounded_channel::<(InstanceId, InstanceCommand)>();
        let environment = InstanceEnvironment::new(
            requests_tx,
            context.primary_monitor_scale_factor(),
            fonts.clone(),
        );

        let mut instance_manager = InstanceManager::new(environment);
        // We need to use ViewEvent early on, because the EventRouter isn't able to convert events.
        let event_manager = EventManager::<ViewEvent>::default();

        // Start one instance of the first registered application
        let primary_application = env
            .applications
            .get_named(&env.primary_application)
            .expect("No primary application");

        instance_manager.spawn(
            primary_application,
            CreationMode::New(InstanceParameters::new()),
        )?;

        // First wait for the initial view that's being created.

        let Some((primary_instance, InstanceCommand::CreateView(creation_info))) =
            requests_rx.recv().await
        else {
            bail!("Did not or received an unexpected request from the application");
        };

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

        let mut system = DesktopSystem::new(
            env,
            project,
            (primary_instance, creation_info.clone()),
            default_size,
            &scene,
            &mut instance_manager,
            renderer.geometry(),
        )?;
        // presenter.present_primary_instance(primary_instance, &creation_info, &scene)?;

        system.update_effects(false, &scene, &mut fonts.lock())?;

        // presenter.layout(creation_info.size(), false, &scene, &mut fonts.lock());
        instance_manager.add_view(primary_instance, &creation_info);

        Ok(Self {
            scene,
            renderer,
            window,
            system,
            event_manager,
            instance_manager,
            instance_commands: requests_rx,
            context,
            fonts,
        })
    }

    pub async fn run(mut self) -> Result<()> {
        loop {
            tokio::select! {
                Some((instance_id, request)) = self.instance_commands.recv() => {
                    self.handle_instance_command(instance_id, request)?;
                }

                shell_event = self.context.wait_for_shell_event() => {
                    let event = shell_event?;

                    match event {
                        ShellEvent::WindowEvent(_window_id, window_event) => {
                            if let Some(view_event) = ViewEvent::from_window_event(&window_event)
                                && let Some(input_event) = self.event_manager.add_event(
                                ExternalEvent::new(ViewId::from(Uuid::nil()), view_event, Instant::now())
                            ) {
                               let cmd = self.system.process_input_event(
                                    &input_event,
                                    &self.instance_manager,
                                    self.renderer.geometry(),
                                )?;
                                self.system.transact(cmd, &self.scene, &mut self.instance_manager, self.renderer.geometry())?;
                                self.system.update_effects(true, &self.scene, &mut self.fonts.lock())?;
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
                    info!("Instance ended: {instance_id:?}");
                    // Hiding is done on shutdown, but what if it's ended by itself?
                    // self.presenter.hide_instance(instance_id)?;


                    // Feature: Display the error to the user?

                    if let Err(e) = instance_result {
                        log::error!("Instance error: {e:?}");
                    }

                    // If all instances have finished, exit
                    if self.instance_manager.is_empty() {
                        return Ok(());
                    }
                }

                else => {
                    return Ok(());
                }
            }

            // Get the camera, build the frame, and submit it to the renderer.
            {
                let camera = self.system.interaction.camera();
                let mut frame = self.scene.begin_frame().with_camera(camera);
                if self.instance_manager.effective_pacing() == RenderPacing::Smooth {
                    frame = frame.with_pacing(RenderPacing::Smooth);
                }
                frame.submit_to(&mut self.renderer)?;
            }
        }
    }

    fn handle_instance_command(
        &mut self,
        instance: InstanceId,
        command: InstanceCommand,
    ) -> Result<()> {
        match command {
            InstanceCommand::CreateView(info) => {
                self.instance_manager.add_view(instance, &info);
                self.system.present_view(instance, &info)?;

                let focused = self.system.interaction.focused();
                // If this instance is currently focused and the new view is primary, make it
                // foreground so that the view is focused.
                if matches!(focused.last(), Some(DesktopTarget::Instance(..)))
                    && focused.instance() == Some(instance)
                    && info.role == ViewRole::Primary
                {
                    let view_focus = focused.clone().join(DesktopTarget::View(info.id));
                    let cmd = self.system.focus(view_focus, &self.instance_manager)?;
                    assert!(cmd.is_none());
                }
            }
            InstanceCommand::DestroyView(id, collector) => {
                self.system.hide_view((instance, id).into())?;
                self.instance_manager.remove_view((instance, id).into());
                // Feature: Don't push the remaining changes immediately and fade the remaining
                // visuals out. (We do have the root location and should be able to do at least
                // alpha blending over that in the future).
                self.scene.accumulate_changes(collector.take_all());
                // Now the collector should not have any references.
                let refs = Arc::strong_count(&collector);
                if refs > 1 {
                    log::error!(
                        "Destroyed view's change collector contains {} unexpected references. Are there pending Visuals / Handles?",
                        refs - 1
                    );
                };
            }
            InstanceCommand::View(view_id, command) => {
                self.handle_view_command((instance, view_id).into(), command)?;
            }
        }
        Ok(())
    }

    fn handle_view_command(&mut self, view: ViewPath, command: ViewCommand) -> Result<()> {
        match command {
            ViewCommand::Render { submission } => {
                self.instance_manager
                    .update_view_pacing(view, submission.pacing)
                    .context("render / update_view_pacing")?;
                self.scene.accumulate_changes(submission.changes);
            }
            ViewCommand::Resize(_) => {
                todo!("Resize is unsupported");
            }
            ViewCommand::SetTitle(title) => {
                self.window.set_title(&title);
            }
            ViewCommand::SetCursor(icon) => {
                self.window.set_cursor(icon);
            }
        }
        Ok(())
    }
}
