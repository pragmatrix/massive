use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use log::info;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use massive_applications::{
    CreationMode, InstanceCommand, InstanceEnvironment, InstanceEvent, InstanceId,
    InstanceParameters, ViewCommand, ViewEvent,
};
use massive_input::EventManager;
use massive_renderer::RenderPacing;
use massive_shell::{ApplicationContext, FontManager, Scene, ShellEvent};
use massive_shell::{AsyncWindowRenderer, ShellWindow};

use crate::DesktopEnvironment;
use crate::desktop_system::{DesktopCommand, DesktopSystem, ProjectCommand};
use crate::event_sourcing::Transaction;
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::projects::{
    GroupId, LaunchGroup, LaunchGroupContents, LaunchGroupProperties, LaunchProfile,
    LaunchProfileId, Launcher, LayoutDirection, Project, ProjectConfiguration, ScopedTag,
};

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

        instance_manager.add_view(primary_instance, &creation_info);

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

        let desktop_groups = desktop_groups();

        // Architecture: Providing the root group here is conceptually wrong I guess, because it
        // does not exist yet.
        let mut system = DesktopSystem::new(env, fonts.clone(), default_size, &scene)?;

        let desktop_groups_transaction = desktop_groups.transaction.map(DesktopCommand::Project);

        // Add the project under the desktop group.
        let project_setup_transaction =
            project_to_transaction(None, &project).map(DesktopCommand::Project);

        let primary_view_transaction: Transaction<_> = [
            // Present the primary instance / view
            DesktopCommand::PresentInstance {
                launcher: desktop_groups.primary_launcher,
                instance: primary_instance,
            },
            DesktopCommand::PresentView(primary_instance, creation_info),
        ]
        .into();

        system.transact(
            desktop_groups_transaction + project_setup_transaction + primary_view_transaction,
            &scene,
            &mut instance_manager,
        )?;

        system.update_effects(false, true)?;

        Ok(Self {
            scene,
            renderer,
            window,
            system,
            event_manager,
            instance_manager,
            instance_commands: requests_rx,
            context,
        })
    }

    pub async fn run(mut self) -> Result<()> {
        loop {
            tokio::select! {
                Some((instance_id, request)) = self.instance_commands.recv() => {
                    self.process_instance_command(instance_id, request)?;
                }

                shell_event = self.context.wait_for_shell_event() => {
                    let event = shell_event?;

                    match event {
                        ShellEvent::WindowEvent(_window_id, window_event) => {
                            if let Some(view_event) = ViewEvent::from_window_event(&window_event)
                                && let Some(input_event) = self.event_manager.add_event(view_event, Instant::now()) {
                               let cmd = self.system.process_input_event(
                                    &input_event,
                                    &self.instance_manager,
                                    self.renderer.geometry(),
                                )?;
                                self.system.transact(cmd, &self.scene, &mut self.instance_manager)?;

                                let allow_camera_movements = !input_event.any_buttons_pressed();
                                self.system.update_effects(true, allow_camera_movements)?;
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
                let camera = self.system.camera();
                let mut frame = self.scene.begin_frame().with_camera(camera);
                if self.instance_manager.effective_pacing() == RenderPacing::Smooth {
                    frame = frame.with_pacing(RenderPacing::Smooth);
                }
                frame.submit_to(&mut self.renderer)?;
            }
        }
    }

    fn process_instance_command(
        &mut self,
        instance: InstanceId,
        command: InstanceCommand,
    ) -> Result<()> {
        match command {
            InstanceCommand::CreateView(info) => {
                self.instance_manager.add_view(instance, &info);
                self.system.transact(
                    DesktopCommand::PresentView(instance, info),
                    &self.scene,
                    &mut self.instance_manager,
                )?;
            }
            InstanceCommand::DestroyView(id, collector) => {
                self.system.transact(
                    DesktopCommand::HideView((instance, id).into()),
                    &self.scene,
                    &mut self.instance_manager,
                )?;
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

#[derive(Debug)]
struct DesktopGroups {
    primary_launcher: LaunchProfileId,
    transaction: Transaction<ProjectCommand>,
}

fn desktop_groups() -> DesktopGroups {
    let mut cmds = Vec::new();

    let primary_group = GroupId::new();
    let primary_launcher = LaunchProfileId::new();

    cmds.push(ProjectCommand::AddLaunchGroup {
        parent: None,
        id: primary_group,
        properties: LaunchGroupProperties {
            name: "TopBand".into(),
            tag: ScopedTag::new("", ""),
            layout: LayoutDirection::Horizontal,
        },
    });
    cmds.push(ProjectCommand::AddLauncher {
        group: primary_group,
        id: primary_launcher,
        profile: LaunchProfile {
            name: "Primary / Local".into(),
            params: Default::default(),
            tags: Default::default(),
        },
    });

    DesktopGroups {
        primary_launcher,
        transaction: cmds.into(),
    }
}

fn project_to_transaction(
    parent: Option<GroupId>,
    project: &Project,
) -> Transaction<ProjectCommand> {
    let mut commands = Vec::new();

    commands.push(ProjectCommand::SetStartupProfile(project.start));

    launch_group_commands(parent, &project.root, &mut commands);

    commands.into()
}

fn launch_group_commands(
    parent: Option<GroupId>,
    group: &LaunchGroup,
    commands: &mut Vec<ProjectCommand>,
) {
    commands.push(ProjectCommand::AddLaunchGroup {
        parent,
        id: group.id,
        properties: group.properties.clone(),
    });

    match &group.contents {
        LaunchGroupContents::Groups(launch_groups) => {
            for launch_group in launch_groups {
                launch_group_commands(Some(group.id), launch_group, commands);
            }
        }
        LaunchGroupContents::Launchers(launchers) => {
            for launcher in launchers {
                launcher_commands(group.id, launcher, commands)
            }
        }
    }
}

fn launcher_commands(group: GroupId, launcher: &Launcher, commands: &mut Vec<ProjectCommand>) {
    commands.push(ProjectCommand::AddLauncher {
        group,
        id: launcher.id,
        profile: launcher.profile.clone(),
    })
}
