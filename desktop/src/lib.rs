use std::{collections::HashMap, future::Future, pin::Pin, time::Instant};

use derive_more::Debug;
use tokio::sync::mpsc::unbounded_channel;
use uuid::Uuid;
use winit::dpi::LogicalSize;

use massive_applications::{
    CreationMode, InstanceCommand, InstanceContext, InstanceEvent, InstanceId, ViewEvent, ViewId,
};
use massive_geometry::{Point, Point3};
use massive_input::{EventManager, ExternalEvent};
use massive_shell::{ApplicationContext, Result, ShellEvent};

mod instance_manager;
mod view_manager;

use instance_manager::InstanceManager;
use view_manager::ViewManager;

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
        let _scene = context.new_scene();

        let (requests_tx, mut requests_rx) = unbounded_channel::<(InstanceId, InstanceCommand)>();
        let mut app_manager =
            InstanceManager::new(context.animation_coordinator().clone(), requests_tx);
        let mut view_manager = ViewManager::new();
        let mut event_manager = EventManager::<winit::event::WindowEvent>::default();

        // Start one instance of the first registered application
        if let Some(app) = self.applications.values().next() {
            app_manager.spawn(app, CreationMode::New)?;
        }

        loop {
            tokio::select! {
                Some((instance_id, request)) = requests_rx.recv() => {
                    match request {
                        InstanceCommand::CreateView(info) => {
                            view_manager.add_view(instance_id, info);
                        }
                        InstanceCommand::DestroyView(id) => {
                            view_manager.remove_view(instance_id, id);
                        }
                        InstanceCommand::View(_id, _command) => {
                            // TODO: Handle view commands (Redraw, Resize)
                        }
                    }
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
                                    &view_manager,
                                    &mut renderer,
                                    &app_manager,
                                )?;
                            }
                        }
                        ShellEvent::ApplyAnimations(_) => {
                            // TODO: Handle animation updates
                        }
                    }
                }

                Some(join_result) = app_manager.join_set.join_next() => {
                    let (instance_id, instance_result) = join_result
                        .unwrap_or_else(|e| (InstanceId::from(Uuid::nil()), Err(anyhow::anyhow!("Instance panicked: {}", e))));

                    app_manager.remove_instance(instance_id);
                    view_manager.remove_instance_views(instance_id);

                    // If any instance fails, return the error
                    instance_result?;

                    // If all instances have finished, exit
                    if app_manager.is_empty() {
                        return Ok(());
                    }
                }

                else => {
                    return Ok(());
                }
            }
        }
    }

    /// Hit test at a 2D screen point and return the topmost view with its local position.
    fn hit_test_at_point(
        screen_pos: Point,
        view_manager: &ViewManager,
        renderer: &mut massive_shell::AsyncWindowRenderer,
    ) -> Option<(ViewId, InstanceId, Point3)> {
        let mut hits = Vec::new();

        for (view_id, view_info) in view_manager.views() {
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
                    hits.push((*view_id, view_info.instance_id, local_pos));
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

    /// Hit test using the cursor position from the input event.
    fn hit_test_from_event(
        input_event: &massive_input::Event<winit::event::WindowEvent>,
        view_manager: &ViewManager,
        renderer: &mut massive_shell::AsyncWindowRenderer,
    ) -> Option<(ViewId, InstanceId, Point3)> {
        let pos = input_event.pos()?;
        Self::hit_test_at_point(pos, view_manager, renderer)
    }

    fn handle_input_event(
        input_event: &massive_input::Event<winit::event::WindowEvent>,
        view_manager: &ViewManager,
        renderer: &mut massive_shell::AsyncWindowRenderer,
        app_manager: &InstanceManager,
    ) -> Result<()> {
        use winit::event::WindowEvent;

        // For events that need hit testing, use position from input_event
        let send_to_view = |view_id: ViewId, instance_id: InstanceId, event: ViewEvent| {
            app_manager.send_event(instance_id, InstanceEvent::View(view_id, event))
        };

        // Get the underlying WindowEvent
        let window_event = input_event.event();

        match window_event {
            WindowEvent::CursorMoved { device_id, .. } => {
                // Use position from input_event which tracks it
                if let Some((view_id, instance_id, local_pos)) =
                    Self::hit_test_from_event(input_event, view_manager, renderer)
                {
                    send_to_view(
                        view_id,
                        instance_id,
                        ViewEvent::CursorMoved {
                            device_id: *device_id,
                            position: (local_pos.x, local_pos.y),
                        },
                    )?;
                }
            }
            WindowEvent::CursorEntered { device_id } => {
                if let Some((view_id, instance_id, _)) =
                    Self::hit_test_from_event(input_event, view_manager, renderer)
                {
                    send_to_view(
                        view_id,
                        instance_id,
                        ViewEvent::CursorEntered {
                            device_id: *device_id,
                        },
                    )?;
                }
            }
            WindowEvent::CursorLeft { device_id } => {
                // Send to all views since we don't know which view the cursor left
                for (view_id, view_info) in view_manager.views() {
                    send_to_view(
                        *view_id,
                        view_info.instance_id,
                        ViewEvent::CursorLeft {
                            device_id: *device_id,
                        },
                    )?;
                }
            }
            WindowEvent::MouseInput {
                device_id,
                state,
                button,
                ..
            } => {
                if let Some((view_id, instance_id, _)) =
                    Self::hit_test_from_event(input_event, view_manager, renderer)
                {
                    send_to_view(
                        view_id,
                        instance_id,
                        ViewEvent::MouseInput {
                            device_id: *device_id,
                            state: *state,
                            button: *button,
                        },
                    )?;
                }
            }
            WindowEvent::MouseWheel {
                device_id,
                delta,
                phase,
                ..
            } => {
                if let Some((view_id, instance_id, _)) =
                    Self::hit_test_from_event(input_event, view_manager, renderer)
                {
                    send_to_view(
                        view_id,
                        instance_id,
                        ViewEvent::MouseWheel {
                            device_id: *device_id,
                            delta: *delta,
                            phase: *phase,
                        },
                    )?;
                }
            }
            WindowEvent::KeyboardInput {
                device_id,
                event,
                is_synthetic,
            } => {
                // TODO: Send to focused view when focus management is implemented
                // For now, send to all views or skip
                let _ = (device_id, event, is_synthetic);
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                // Send modifiers to all views
                for (view_id, view_info) in view_manager.views() {
                    send_to_view(
                        *view_id,
                        view_info.instance_id,
                        ViewEvent::ModifiersChanged(*modifiers),
                    )?;
                }
            }
            WindowEvent::Ime(ime) => {
                // TODO: Send to focused view when focus management is implemented
                let _ = ime;
            }
            WindowEvent::DroppedFile(path) => {
                if let Some((view_id, instance_id, _)) =
                    Self::hit_test_from_event(input_event, view_manager, renderer)
                {
                    send_to_view(view_id, instance_id, ViewEvent::DroppedFile(path.clone()))?;
                }
            }
            WindowEvent::HoveredFile(path) => {
                if let Some((view_id, instance_id, _)) =
                    Self::hit_test_from_event(input_event, view_manager, renderer)
                {
                    send_to_view(view_id, instance_id, ViewEvent::HoveredFile(path.clone()))?;
                }
            }
            WindowEvent::HoveredFileCancelled => {
                // Send to all views
                for (view_id, view_info) in view_manager.views() {
                    send_to_view(
                        *view_id,
                        view_info.instance_id,
                        ViewEvent::HoveredFileCancelled,
                    )?;
                }
            }
            WindowEvent::CloseRequested => {
                // Send to all views
                for (view_id, view_info) in view_manager.views() {
                    send_to_view(*view_id, view_info.instance_id, ViewEvent::CloseRequested)?;
                }
            }
            _ => {
                // Ignore other events
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct Application {
    name: String,
    #[debug(skip)]
    run: RunInstanceBox,
}

type RunInstanceBox = Box<
    dyn Fn(InstanceContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>
        + Send
        + Sync
        + 'static,
>;

impl Application {
    pub fn new<F, R>(name: impl Into<String>, run: F) -> Self
    where
        F: Fn(InstanceContext) -> R + Send + Sync + 'static,
        R: Future<Output = Result<()>> + Send + 'static,
    {
        let name = name.into();
        let run_boxed = Box::new(
            move |ctx: InstanceContext| -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
                Box::pin(run(ctx))
            },
        );

        Self {
            name,
            run: run_boxed,
        }
    }
}
