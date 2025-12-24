use std::time::Instant;

use anyhow::{anyhow, bail};
use tokio::sync::mpsc::unbounded_channel;
use winit::event;

use massive_applications::{
    CreationMode, InstanceCommand, InstanceEnvironment, InstanceEvent, InstanceId, Scene,
    ViewCommand, ViewRole,
};
use massive_input::{EventManager, ExternalEvent};
use massive_renderer::{FontManager, RenderPacing};
use massive_shell::{ApplicationContext, Result, ShellEvent, ShellWindow};

use crate::{
    Application, UI, UiCommand,
    application_registry::ApplicationRegistry,
    desktop_presenter::DesktopPresenter,
    instance_manager::{InstanceManager, ViewPath},
};

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
        let mut presenter = DesktopPresenter::default();
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
        let primary_view = creation_info.id;

        let window = context.new_window(creation_info.size()).await?;
        let mut renderer = window
            .renderer()
            .with_shapes()
            .with_text(fonts.clone())
            .with_background_color(massive_geometry::Color::BLACK)
            .build()
            .await?;

        let scene = context.new_scene();

        presenter.present_primary_instance(primary_instance, &creation_info, &scene)?;
        presenter.layout(false);
        instance_manager.add_view(primary_instance, &creation_info);
        let mut ui = UI::new(
            (primary_instance, primary_view).into(),
            &instance_manager,
            &presenter,
            &scene,
        )?;

        loop {
            tokio::select! {
                Some((instance_id, request)) = requests_rx.recv() => {
                    Self::handle_instance_command(&mut instance_manager, &mut presenter, &mut ui, &scene, &window, instance_id, request)?;
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
                                    &instance_manager,
                                    renderer.geometry(),
                                )?;

                                self.handle_ui_command(cmd, &mut instance_manager, &mut presenter, &mut ui, &scene)?;
                            }

                            renderer.resize_redraw(&window_event)?;
                        }
                        ShellEvent::ApplyAnimations(_) => {
                            // Performance: Not every instance needs that, only the ones animating.
                            instance_manager.broadcast_event(InstanceEvent::ApplyAnimations);
                            presenter.apply_animations();
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

            let mut frame = scene.begin_frame().with_camera(ui.camera());
            if instance_manager.effective_pacing() == RenderPacing::Smooth {
                frame = frame.with_pacing(RenderPacing::Smooth);
            }
            frame.submit_to(&mut renderer)?;
        }
    }

    fn handle_ui_command(
        &self,
        cmd: UiCommand,
        instance_manager: &mut InstanceManager,
        presenter: &mut DesktopPresenter,
        ui: &mut UI,
        scene: &Scene,
    ) -> Result<()> {
        match cmd {
            UiCommand::None => {}
            UiCommand::StartInstance {
                application,
                originating_instance,
            } => {
                let application = self
                    .applications
                    .get_named(&application)
                    .ok_or(anyhow!("Internal error, application not registered"))?;

                let instance = instance_manager.spawn(application, CreationMode::New)?;
                presenter.present_instance(instance, originating_instance, scene)?;
                // Window focus is faked here.
                ui.make_foreground(instance, instance_manager, presenter)?;
                presenter.layout(true);
            }
            UiCommand::MakeForeground { instance } => {
                ui.make_foreground(instance, instance_manager, presenter)?;
            }
            UiCommand::StopInstance { instance } => instance_manager.stop(instance)?,
        }

        Ok(())
    }

    fn handle_instance_command(
        instance_manager: &mut InstanceManager,
        presenter: &mut DesktopPresenter,
        ui: &mut UI,
        scene: &Scene,
        window: &ShellWindow,
        instance: InstanceId,
        command: InstanceCommand,
    ) -> Result<()> {
        match command {
            InstanceCommand::CreateView(info) => {
                instance_manager.add_view(instance, &info);
                presenter.present_view(instance, &info)?;
                // If this instance is currently focused and the new view is primary, make it
                // foreground so that the view is focused.
                if ui.focused_instance() == Some(instance) && info.role == ViewRole::Primary {
                    ui.make_foreground(instance, instance_manager, presenter)?;
                }
            }
            InstanceCommand::DestroyView(id) => {
                presenter.hide_view(id)?;
                instance_manager.remove_view((instance, id).into());
            }
            InstanceCommand::View(view_id, command) => {
                Self::handle_view_command(
                    instance_manager,
                    scene,
                    window,
                    (instance, view_id).into(),
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
        view: ViewPath,
        command: ViewCommand,
    ) -> Result<()> {
        match command {
            ViewCommand::Render { submission } => {
                instance_manager.update_view_pacing(view, submission.pacing)?;
                scene.accumulate_changes(submission.changes);
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
