use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use euclid::default;
use log::info;
use massive_geometry::SizePx;
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

use crate::desktop_presenter::DesktopTarget;
use crate::desktop_system::DesktopSystem;
use crate::projects::Project;
use crate::{
    DesktopEnvironment, DesktopInteraction, UserIntent,
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
    presenter: DesktopSystem,

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

        let mut presenter = DesktopSystem::new(project, default_size, &scene)?;
        // presenter.present_primary_instance(primary_instance, &creation_info, &scene)?;

        // Present the default terminal inside of the top band.
        {
            presenter.present_instance(
                &[DesktopTarget::Desktop, DesktopTarget::TopBand]
                    .to_vec()
                    .into(),
                primary_instance,
                None,
                default_size,
                &scene,
            )?;
            presenter.present_view(primary_instance, &creation_info)?;
        }

        presenter.layout(creation_info.size(), false, &scene, &mut fonts.lock());
        instance_manager.add_view(primary_instance, &creation_info);

        let ui = DesktopInteraction::new(
            [
                DesktopTarget::Desktop,
                DesktopTarget::TopBand,
                DesktopTarget::Instance(primary_instance),
                DesktopTarget::View(primary_view),
            ]
            .to_vec()
            .into(),
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
                let camera = self.interaction.camera();
                let mut frame = self.scene.begin_frame().with_camera(camera);
                if self.instance_manager.effective_pacing() == RenderPacing::Smooth {
                    frame = frame.with_pacing(RenderPacing::Smooth);
                }
                frame.submit_to(&mut self.renderer)?;
            }
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
            UserIntent::StartInstance { parameters } => {
                // Feature: Support starting non-primary applications.
                let application = self
                    .env
                    .applications
                    .get_named(&self.env.primary_application)
                    .ok_or(anyhow!("Internal error, application not registered"))?;

                let instance = self
                    .instance_manager
                    .spawn(application, CreationMode::New(parameters))?;

                let focused = self.interaction.focused();
                let originating_instance = focused.instance();

                let presented_instance_path = self.presenter.present_instance(
                    focused,
                    instance,
                    originating_instance,
                    self.primary_instance_panel_size,
                    &self.scene,
                )?;

                let intent = self.interaction.focus(
                    presented_instance_path,
                    &self.instance_manager,
                    &mut self.presenter,
                )?;

                assert_eq!(intent, UserIntent::None);

                // Performance: We might not need a global re-layout, if we present an instance
                // to the project's band (This has to work incremental some day).
                self.presenter.layout(
                    self.primary_instance_panel_size,
                    true,
                    &self.scene,
                    &mut self.fonts.lock(),
                );
            }
            UserIntent::StopInstance { instance } => {
                // Remove the instance from the focus first.
                //
                // Detail: This causes an unfocus event sent to the instance's view. I don't think
                // this should happen on teardown.
                let focus = self.interaction.focused();
                if let Some(focused_instance) = self.interaction.focused().instance()
                    && focused_instance == instance
                {
                    let instance_parent = focus.instance_parent().expect("Internal error: Instance parent failed even though instance() returned one.");
                    let intent = self.interaction.focus(
                        instance_parent,
                        &self.instance_manager,
                        &mut self.presenter,
                    )?;
                    assert_eq!(intent, UserIntent::None);
                }

                // Trigger the shutdown.
                self.instance_manager.trigger_shutdown(instance)?;

                // We hide the instance as soon we trigger a shutdown so that they can't be in the
                // navigation tree anymore.
                self.presenter.hide_instance(instance)?;

                // Refocus the cursor since it may be pointing to the removed instance.
                let intent = self.interaction.refocus_pointer(
                    &self.instance_manager,
                    &mut self.presenter,
                    self.renderer.geometry(),
                )?;
                // No intent on refocusing allowed.
                assert_eq!(intent, UserIntent::None);
            }
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

                let focused = self.interaction.focused();
                // If this instance is currently focused and the new view is primary, make it
                // foreground so that the view is focused.
                if matches!(focused.last(), Some(DesktopTarget::Instance(..)))
                    && focused.instance() == Some(instance)
                    && info.role == ViewRole::Primary
                {
                    let view_focus = focused.clone().join(DesktopTarget::View(info.id));
                    let intent = self.interaction.focus(
                        view_focus,
                        &self.instance_manager,
                        &mut self.presenter,
                    )?;

                    assert_eq!(intent, UserIntent::None)
                }
            }
            InstanceCommand::DestroyView(id, collector) => {
                self.presenter.hide_view((instance, id).into())?;
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
