use std::time::Instant;

use anyhow::{anyhow, bail};
use tokio::sync::mpsc::unbounded_channel;
use winit::{
    dpi::PhysicalSize,
    event::{self},
};

use massive_applications::{
    CreationMode, InstanceCommand, InstanceEnvironment, InstanceEvent, InstanceId, Options,
    RenderPacing, Scene, ViewCommand, ViewId,
};
use massive_input::{EventManager, ExternalEvent};
use massive_renderer::FontManager;
use massive_shell::{ApplicationContext, Result, ShellEvent, ShellWindow};

mod application_registry;
mod focus_manager;
mod instance_manager;
mod ui;

pub use application_registry::Application;
pub use focus_manager::*;
pub use ui::*;

use crate::application_registry::ApplicationRegistry;
use instance_manager::InstanceManager;

#[derive(Debug)]
pub struct Desktop {
    primary_application: String,
    applications: ApplicationRegistry,
}

impl Desktop {
    pub fn new(applications: Vec<Application>) -> Self {
        Self {
            primary_application: applications
                .first()
                .expect("No primary application")
                .name
                .clone(),
            applications: ApplicationRegistry::new(applications),
        }
    }

    pub async fn run(self, mut context: ApplicationContext) -> Result<()> {
        // Create the font manager - shared between desktop and instances
        let fonts = FontManager::system();

        let (requests_tx, mut requests_rx) = unbounded_channel::<(InstanceId, InstanceCommand)>();
        let mut ui = UI::new();
        let environment = InstanceEnvironment::new(
            requests_tx,
            context.primary_monitor_scale_factor(),
            fonts.clone(),
        );
        let mut instance_manager = InstanceManager::new(environment);
        let mut event_manager = EventManager::<event::WindowEvent>::default();

        // Start one instance of the first registered application
        let primary_application = self
            .applications
            .get_named(&self.primary_application)
            .expect("No primary application");

        instance_manager.spawn(primary_application, CreationMode::New)?;

        // First wait for the initial view that's being created.

        let Some((primary_instance, InstanceCommand::CreateView(creation_info))) =
            requests_rx.recv().await
        else {
            bail!("Did not or received an unexpected request from the application");
        };
        instance_manager.add_view(primary_instance, creation_info.clone());

        // Then create the window, renderer, and scene.

        let (width, height) = creation_info.size;
        // Create a window and renderer
        let window = context.new_window(PhysicalSize::new(width, height)).await?;
        let mut renderer = window
            .renderer()
            .with_shapes()
            .with_text(fonts.clone())
            .with_background_color(massive_geometry::Color::BLACK)
            .build()
            .await?;
        let scene = context.new_scene();

        loop {
            tokio::select! {
                Some((instance_id, request)) = requests_rx.recv() => {
                    Self::handle_instance_command(&mut instance_manager, &scene, &window, instance_id, request)?;
                }

                shell_event = context.wait_for_shell_event() => {
                    let event = shell_event?;

                    match event {
                        ShellEvent::WindowEvent(window_id, window_event) => {
                            // Process through EventManager
                            if let Some(input_event) = event_manager.add_event(
                                ExternalEvent::from_window_event(window_id, window_event.clone(), Instant::now())
                            ) {
                                let cmd = ui.handle_input_event(
                                    &input_event,
                                    primary_instance,
                                    &mut instance_manager,
                                    renderer.geometry(),
                                )?;

                                self.handle_ui_command(&mut instance_manager, cmd)?;
                            }

                            renderer.resize_redraw(&window_event)?;
                        }
                        ShellEvent::ApplyAnimations(_) => {
                            instance_manager.broadcast_event(InstanceEvent::ApplyAnimations);
                        }
                    }
                }

                Ok((_instance_id, instance_result)) = instance_manager.join_next() => {

                    // If any instance fails, return the error
                    instance_result?;

                    // If all instances have finished, exit
                    if instance_manager.is_empty() {
                        return Ok(());
                    }
                }

                else => {
                    return Ok(());
                }
            }

            // Render all accumulated changes with the appropriate pacing
            let options = if instance_manager.effective_pacing() == RenderPacing::Smooth {
                Some(Options::ForceSmoothRendering)
            } else {
                None
            };
            scene.render_to_with_options(&mut renderer, options)?;
        }
    }

    fn handle_ui_command(
        &self,
        instance_manager: &mut InstanceManager,
        cmd: UiCommand,
    ) -> Result<()> {
        match cmd {
            UiCommand::None => {}
            UiCommand::NewInstance { application } => {
                let application = self
                    .applications
                    .get_named(&application)
                    .ok_or(anyhow!("Internal error, application not registered"))?;

                instance_manager.spawn(application, CreationMode::New)?;
            }
            UiCommand::CloseInstance { instance } => instance_manager.stop(instance)?,
        }

        Ok(())
    }

    fn handle_instance_command(
        instance_manager: &mut InstanceManager,
        scene: &Scene,
        window: &ShellWindow,
        instance_id: InstanceId,
        command: InstanceCommand,
    ) -> Result<()> {
        match command {
            InstanceCommand::CreateView(info) => {
                instance_manager.add_view(instance_id, info);
            }
            InstanceCommand::DestroyView(id) => {
                instance_manager.remove_view(instance_id, id);
            }
            InstanceCommand::View(view_id, command) => {
                Self::handle_view_command(
                    instance_manager,
                    scene,
                    window,
                    instance_id,
                    view_id,
                    command,
                )?;
            }
        }
        Ok(())
    }

    fn handle_view_command(
        instance_manager: &mut InstanceManager,
        scene: &Scene,
        window: &ShellWindow,
        instance_id: InstanceId,
        view_id: ViewId,
        command: ViewCommand,
    ) -> Result<()> {
        match command {
            ViewCommand::Render { changes, pacing } => {
                instance_manager.update_view_pacing(instance_id, view_id, pacing)?;
                scene.push_changes(changes);
            }
            ViewCommand::Resize(_) => {
                todo!("Resize is unsupported");
            }
            ViewCommand::SetTitle(title) => {
                window.set_title(&title);
            }
            ViewCommand::SetCursor(icon) => {
                window.set_cursor(icon);
            }
        }
        Ok(())
    }
}
