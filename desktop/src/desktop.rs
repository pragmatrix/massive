use std::time::Instant;

use anyhow::{Result, anyhow, bail};
use massive_geometry::SizePx;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use uuid::Uuid;

use massive_applications::{
    CreationMode, InstanceCommand, InstanceEnvironment, InstanceEvent, InstanceId, ViewCommand,
    ViewEvent, ViewId, ViewRole,
};
use massive_input::{EventManager, ExternalEvent};
use massive_renderer::RenderPacing;
use massive_shell::{ApplicationContext, FontManager, Scene, ShellEvent};
use massive_shell::{AsyncWindowRenderer, ShellWindow};

use crate::desktop_presenter::DesktopFocusPath;
use crate::projects::Project;
use crate::{
    DesktopEnvironment, DesktopInteraction, DesktopPresenter, UserIntent,
    instance_manager::{InstanceManager, ViewPath},
    projects::ProjectConfiguration,
};

#[derive(Debug)]
pub struct Desktop {
    interaction: DesktopInteraction,
    scene: Scene,
    renderer: AsyncWindowRenderer,
    window: ShellWindow,
    primary_instance_panel_size: SizePx,
    presenter: DesktopPresenter,

    event_manager: EventManager<ViewEvent>,

    instance_manager: InstanceManager,
    instance_commands: UnboundedReceiver<(InstanceId, InstanceCommand)>,
    context: ApplicationContext,
    env: DesktopEnvironment,
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
        let mut presenter = DesktopPresenter::new(project, &scene);
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

        instance_manager.spawn(primary_application, CreationMode::New)?;

        // First wait for the initial view that's being created.

        let Some((primary_instance, InstanceCommand::CreateView(creation_info))) =
            requests_rx.recv().await
        else {
            bail!("Did not or received an unexpected request from the application");
        };

        // Currently we can't target views directly, the focus system is targeting only instances
        // and their primary view.
        let primary_view = creation_info.id;
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

        presenter.present_primary_instance(primary_instance, &creation_info, &scene)?;
        presenter.layout(creation_info.size(), false, &scene, &mut fonts.lock());
        instance_manager.add_view(primary_instance, &creation_info);

        let ui = DesktopInteraction::new(
            DesktopFocusPath::from_instance_and_view(primary_instance, primary_view),
            &instance_manager,
            &mut presenter,
            &scene,
        )?;

        Ok(Self {
            interaction: ui,
            scene,
            renderer,
            window,
            primary_instance_panel_size: default_size,
            presenter,
            event_manager,
            instance_manager,
            instance_commands: requests_rx,
            context,
            env,
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
                               let cmd = self.interaction.process_input_event(
                                    &input_event,
                                    &self.instance_manager,
                                    &mut self.presenter,
                                    self.renderer.geometry(),
                                )?;
                                self.process_user_intent(cmd)?;
                            }

                            self.renderer.resize_redraw(&window_event)?;
                        }
                        ShellEvent::ApplyAnimations(_) => {
                            // Performance: Not every instance needs that, only the ones animating.
                            self.instance_manager.broadcast_event(InstanceEvent::ApplyAnimations);
                            self.presenter.apply_animations();
                        }
                    }
                }

                Ok((_instance_id, instance_result)) = self.instance_manager.join_next() => {

                    // If any instance fails, return the error
                    instance_result?;

                    // If all instances have finished, exit
                    if self.instance_manager.is_empty() {
                        return Ok(());
                    }
                }

                else => {
                    return Ok(());
                }
            }

            let camera = self.interaction.camera();
            let mut frame = self.scene.begin_frame().with_camera(camera);
            if self.instance_manager.effective_pacing() == RenderPacing::Smooth {
                frame = frame.with_pacing(RenderPacing::Smooth);
            }
            frame.submit_to(&mut self.renderer)?;
        }
    }

    fn process_user_intent(&mut self, cmd: UserIntent) -> Result<()> {
        match cmd {
            UserIntent::None => {}
            UserIntent::Focus(path) => {
                assert_eq!(
                    self.interaction
                        .focus(path, &self.instance_manager, &mut self.presenter)?,
                    UserIntent::None
                );
            }
            UserIntent::StartInstance {
                originating_instance,
            } => {
                // Feature: Support starting non-primary applications.
                let application = self
                    .env
                    .applications
                    .get_named(&self.env.primary_application)
                    .ok_or(anyhow!("Internal error, application not registered"))?;

                let instance = self
                    .instance_manager
                    .spawn(application, CreationMode::New)?;

                // Simplify: Use the currently focused instance for determining the originating one.
                let instance_target = self
                    .interaction
                    .focused()
                    .instance_target()
                    .expect("Failed to start an instance without a focused instance target");

                self.presenter.present_instance(
                    instance_target,
                    instance,
                    originating_instance,
                    self.primary_instance_panel_size,
                    &self.scene,
                )?;

                self.interaction.make_foreground(
                    instance,
                    &self.instance_manager,
                    &mut self.presenter,
                )?;
                self.presenter.layout(
                    self.primary_instance_panel_size,
                    true,
                    &self.scene,
                    &mut self.fonts.lock(),
                );
            }
            UserIntent::StopInstance { instance } => self.instance_manager.stop(instance)?,
        }

        Ok(())
    }

    fn handle_instance_command(
        &mut self,
        instance: InstanceId,
        command: InstanceCommand,
    ) -> Result<()> {
        match command {
            InstanceCommand::CreateView(info) => {
                self.instance_manager.add_view(instance, &info);
                self.presenter.present_view(instance, &info)?;
                // If this instance is currently focused and the new view is primary, make it
                // foreground so that the view is focused.
                if self.interaction.focused().instance() == Some(instance)
                    && info.role == ViewRole::Primary
                {
                    self.interaction.make_foreground(
                        instance,
                        &self.instance_manager,
                        &mut self.presenter,
                    )?;
                }
            }
            InstanceCommand::DestroyView(id) => {
                self.presenter.hide_view(id)?;
                self.instance_manager.remove_view((instance, id).into());
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
                    .update_view_pacing(view, submission.pacing)?;
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
