use std::{collections::HashMap, time::Instant};

use tokio::sync::mpsc::unbounded_channel;
use winit::{
    dpi::LogicalSize,
    event::{self, WindowEvent},
};

use massive_applications::{
    CreationMode, InstanceCommand, InstanceEvent, InstanceId, Options, RenderPacing, Scene,
    ViewCommand, ViewEvent, ViewId,
};
use massive_geometry::{Point, Point3};
use massive_input::{Event, EventManager, ExternalEvent};
use massive_shell::{ApplicationContext, AsyncWindowRenderer, Result, ShellEvent};

mod instance_manager;

pub use instance_manager::Application;
use instance_manager::InstanceManager;

#[derive(Debug)]
pub struct Desktop {
    applications: HashMap<String, Application>,
}

impl Desktop {
    pub fn new(applications: Vec<Application>) -> Self {
        Self {
            applications: HashMap::from_iter(applications.into_iter().map(|a| (a.name.clone(), a))),
        }
    }

    pub async fn run(self, mut context: ApplicationContext) -> Result<()> {
        // Create a window and renderer
        let window = context.new_window(LogicalSize::new(1024, 768)).await?;
        let mut renderer = window.renderer().build().await?;
        let scene = context.new_scene();

        let (requests_tx, mut requests_rx) = unbounded_channel::<(InstanceId, InstanceCommand)>();
        let mut instance_manager = InstanceManager::new(requests_tx);
        let mut event_manager = EventManager::<event::WindowEvent>::default();

        // Start one instance of the first registered application
        if let Some(app) = self.applications.values().next() {
            instance_manager.spawn(app, CreationMode::New)?;
        }

        loop {
            tokio::select! {
                Some((instance_id, request)) = requests_rx.recv() => {
                    Self::handle_instance_command(&mut instance_manager, &scene, instance_id, request)?;
                }

                shell_event = context.wait_for_shell_event() => {
                    let event = shell_event?;

                    match event {
                        ShellEvent::WindowEvent(window_id, window_event) => {
                            // Process through EventManager
                            if let Some(input_event) = event_manager.add_event(
                                ExternalEvent::from_window_event(window_id, window_event, Instant::now())
                            ) {
                                Self::handle_input_event(
                                    &input_event,
                                    &mut renderer,
                                    &instance_manager,
                                )?;
                            }
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

    fn handle_instance_command(
        instance_manager: &mut InstanceManager,
        scene: &Scene,
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
                Self::handle_view_command(instance_manager, scene, instance_id, view_id, command)?;
            }
        }
        Ok(())
    }

    fn handle_view_command(
        instance_manager: &mut InstanceManager,
        scene: &Scene,
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
        }
        Ok(())
    }

    fn handle_input_event(
        input_event: &Event<WindowEvent>,
        renderer: &mut AsyncWindowRenderer,
        instance_manager: &InstanceManager,
    ) -> Result<()> {
        let send_to_view = |view_id: ViewId, instance_id: InstanceId, event: ViewEvent| {
            instance_manager.send_event(instance_id, InstanceEvent::View(view_id, event))
        };

        let window_event = input_event.event();
        let hit_result = Self::hit_test_from_event(input_event, instance_manager, renderer);

        match window_event {
            WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::DroppedFile(_)
            | WindowEvent::HoveredFile(_) => {
                if let Some((view_id, instance_id, local_pos)) = hit_result
                    && let Some(view_event) =
                        Self::convert_window_event_to_view_event(window_event, Some(local_pos))
                {
                    send_to_view(view_id, instance_id, view_event)?;
                }
            }
            WindowEvent::CursorLeft { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::HoveredFileCancelled
            | WindowEvent::CloseRequested => {
                for (instance_id, view_id, _view_info) in instance_manager.views() {
                    if let Some(view_event) =
                        Self::convert_window_event_to_view_event(window_event, None)
                    {
                        send_to_view(*view_id, instance_id, view_event)?;
                    }
                }
            }
            WindowEvent::KeyboardInput {
                device_id,
                event,
                is_synthetic,
            } => {
                let _ = (device_id, event, is_synthetic);
            }
            WindowEvent::Ime(ime) => {
                let _ = ime;
            }
            _ => {}
        }

        Ok(())
    }

    fn hit_test_from_event(
        input_event: &Event<WindowEvent>,
        instance_manager: &InstanceManager,
        renderer: &mut AsyncWindowRenderer,
    ) -> Option<(ViewId, InstanceId, Point3)> {
        let pos = input_event.pos()?;
        Self::hit_test_at_point(pos, instance_manager, renderer)
    }

    fn hit_test_at_point(
        screen_pos: Point,
        instance_manager: &InstanceManager,
        renderer: &mut AsyncWindowRenderer,
    ) -> Option<(ViewId, InstanceId, Point3)> {
        let mut hits = Vec::new();

        for (instance_id, view_id, view_info) in instance_manager.views() {
            let location = view_info.location.value();
            let matrix = location.matrix.value();
            let size = view_info.size;

            if let Some(local_pos) = renderer
                .geometry()
                .unproject_to_model_z0(screen_pos, &matrix)
            {
                // Check if the local position is within the view bounds
                if local_pos.x >= 0.0
                    && local_pos.x <= size.0 as f64
                    && local_pos.y >= 0.0
                    && local_pos.y <= size.1 as f64
                {
                    hits.push((*view_id, instance_id, local_pos));
                }
            }
        }

        // Sort by z (descending) to get topmost view first
        hits.sort_by(|a, b| {
            b.2.z
                .partial_cmp(&a.2.z)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        hits.first().copied()
    }

    fn convert_window_event_to_view_event(
        window_event: &WindowEvent,
        pos: Option<Point3>,
    ) -> Option<ViewEvent> {
        match window_event {
            WindowEvent::CursorMoved { device_id, .. } => {
                let local_pos = pos?;
                Some(ViewEvent::CursorMoved {
                    device_id: *device_id,
                    position: (local_pos.x, local_pos.y),
                })
            }
            WindowEvent::CursorEntered { device_id } => Some(ViewEvent::CursorEntered {
                device_id: *device_id,
            }),
            WindowEvent::CursorLeft { device_id } => Some(ViewEvent::CursorLeft {
                device_id: *device_id,
            }),
            WindowEvent::MouseInput {
                device_id,
                state,
                button,
                ..
            } => Some(ViewEvent::MouseInput {
                device_id: *device_id,
                state: *state,
                button: *button,
            }),
            WindowEvent::MouseWheel {
                device_id,
                delta,
                phase,
                ..
            } => Some(ViewEvent::MouseWheel {
                device_id: *device_id,
                delta: *delta,
                phase: *phase,
            }),
            WindowEvent::ModifiersChanged(modifiers) => {
                Some(ViewEvent::ModifiersChanged(*modifiers))
            }
            WindowEvent::DroppedFile(path) => Some(ViewEvent::DroppedFile(path.clone())),
            WindowEvent::HoveredFile(path) => Some(ViewEvent::HoveredFile(path.clone())),
            WindowEvent::HoveredFileCancelled => Some(ViewEvent::HoveredFileCancelled),
            WindowEvent::CloseRequested => Some(ViewEvent::CloseRequested),
            _ => None,
        }
    }
}
