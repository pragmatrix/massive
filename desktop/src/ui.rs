use std::cmp::Ordering;

use anyhow::Result;

use winit::event::WindowEvent;

use massive_applications::{InstanceId, ViewEvent, ViewId, ViewRole};
use massive_geometry::{Point, Point3};
use massive_input::Event;
use massive_renderer::RenderGeometry;

use crate::{FocusManager, FocusTransition, instance_manager::InstanceManager};

#[derive(Debug, Default)]
pub struct UI {
    /// The recently touched view with the cursor / mouse, None if it's the desktop background.
    /// Let's call this mouse focus for now.
    ///
    /// The ids may not exist.
    cursor_focus: Option<CursorFocus>,

    /// Basically the keyboard focus.
    focus_manager: FocusManager,
}

#[derive(Debug)]
struct CursorFocus {
    instance: InstanceId,
    view: ViewId,
    hit_on_view: Point3,
}

impl UI {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle_input_event(
        &mut self,
        input_event: &Event<WindowEvent>,
        primary_instance: InstanceId,
        instance_manager: &mut InstanceManager,
        render_geometry: &RenderGeometry,
    ) -> Result<()> {
        let send_to_view = |instance_id: InstanceId, view_id: ViewId, event: ViewEvent| {
            instance_manager.send_view_event(instance_id, view_id, event)
        };

        let send_window_event = |instance_id, view_id, window_event| {
            if let Some(view_event) = Self::convert_window_event_to_view_event(window_event) {
                send_to_view(instance_id, view_id, view_event)
            } else {
                Ok(())
            }
        };

        let window_event = input_event.event();

        match window_event {
            WindowEvent::Focused(window_focused) => {
                self.make_foreground(primary_instance, *window_focused, instance_manager)?;
            }

            WindowEvent::CursorMoved { device_id, .. } => {
                let any_pressed = input_event
                    .pointing_device_state()
                    .map(|d| d.any_button_pressed())
                    .unwrap_or(false);

                // Change the cursor focus only if there is no button pressed.
                //
                // Robustness: There might be a change of the device here.
                if !any_pressed {
                    let hit_result =
                        Self::hit_test_from_event(input_event, instance_manager, render_geometry);
                    self.cursor_focus = hit_result.map(|(instance, view, point)| CursorFocus {
                        instance,
                        view,
                        hit_on_view: point,
                    });
                } else {
                    // Button is pressed, may update pos only.
                    if let Some(cursor_focus) = &mut self.cursor_focus {
                        let screen_pos = input_event
                            .pos()
                            .expect("A cursor moved event must have a position");
                        if let Some(hit_result) = Self::hit_test_on_view(
                            screen_pos,
                            instance_manager,
                            render_geometry,
                            cursor_focus.instance,
                            cursor_focus.view,
                        ) {
                            cursor_focus.hit_on_view = hit_result
                        } else {
                            // Looks like the view vanished
                            self.cursor_focus = None;
                        }
                    }
                }

                // If there is a current cursor focus, forward the event.
                if let Some(CursorFocus {
                    instance,
                    view,
                    hit_on_view,
                    ..
                }) = self.cursor_focus
                {
                    send_to_view(
                        instance,
                        view,
                        ViewEvent::CursorMoved {
                            device_id: *device_id,
                            position: (hit_on_view.x, hit_on_view.y),
                        },
                    )?
                }
            }

            // Cursor focus
            WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::DroppedFile(_)
            | WindowEvent::HoveredFile(_) => {
                if let Some(CursorFocus { instance, view, .. }) = self.cursor_focus {
                    send_window_event(instance, view, window_event)?;
                }
            }

            // Keyboard focus
            WindowEvent::KeyboardInput { .. } | WindowEvent::Ime(_) => {
                if let Some((instance, view)) = self.focus_manager.focused_view() {
                    send_window_event(instance, view, window_event)?;
                }
            }

            // All views
            WindowEvent::ModifiersChanged(_)
            | WindowEvent::HoveredFileCancelled
            | WindowEvent::CloseRequested
            | WindowEvent::Resized(_) => {
                for (instance, view, _view_info) in instance_manager.views() {
                    send_window_event(instance, view, window_event)?;
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn hit_test_from_event(
        input_event: &Event<WindowEvent>,
        instance_manager: &InstanceManager,
        render_geometry: &RenderGeometry,
    ) -> Option<(InstanceId, ViewId, Point3)> {
        let pos = input_event.pos()?;
        Self::hit_test_at_point(pos, instance_manager, render_geometry)
    }

    fn hit_test_at_point(
        screen_pos: Point,
        instance_manager: &InstanceManager,
        geometry: &RenderGeometry,
    ) -> Option<(InstanceId, ViewId, Point3)> {
        let mut hits = Vec::new();

        for (instance_id, view_id, view_info) in instance_manager.views() {
            let location = view_info.location.value();
            let matrix = location.matrix.value();
            let size = view_info.size;

            if let Some(local_pos) = geometry.unproject_to_model_z0(screen_pos, &matrix) {
                // Check if the local position is within the view bounds
                if local_pos.x >= 0.0
                    && local_pos.x <= size.0 as f64
                    && local_pos.y >= 0.0
                    && local_pos.y <= size.1 as f64
                {
                    hits.push((instance_id, view_id, local_pos));
                }
            }
        }

        // Sort by z (descending) to get topmost view first
        hits.sort_by(|a, b| b.2.z.partial_cmp(&a.2.z).unwrap_or(Ordering::Equal));

        hits.first().copied()
    }

    /// Specifically hit tests on a view without considering its boundaries.
    ///
    /// Meaning that the hit test result may be out of bounds.
    fn hit_test_on_view(
        screen_pos: Point,
        instance_manager: &InstanceManager,
        render_geometry: &RenderGeometry,
        instance_id: InstanceId,
        view_id: ViewId,
    ) -> Option<Point3> {
        let view_info = instance_manager.get_view(instance_id, view_id).ok()?;
        let location = view_info.location.value();
        let matrix = location.matrix.value();
        render_geometry.unproject_to_model_z0(screen_pos, &matrix)
    }

    /// Convert all window events to a matching view event if available, except cursor moved.
    fn convert_window_event_to_view_event(window_event: &WindowEvent) -> Option<ViewEvent> {
        match window_event {
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
            WindowEvent::KeyboardInput {
                device_id,
                event,
                is_synthetic,
            } => Some(ViewEvent::KeyboardInput {
                device_id: *device_id,
                event: event.clone(),
                is_synthetic: *is_synthetic,
            }),
            WindowEvent::Ime(ime) => Some(ViewEvent::Ime(ime.clone())),
            WindowEvent::Focused(focused) => Some(ViewEvent::Focused(*focused)),
            WindowEvent::Resized(size) => Some(ViewEvent::Resized(size.width, size.height)),
            _ => None,
        }
    }

    pub fn make_foreground(
        &mut self,
        instance: InstanceId,
        window_focused: bool,
        instance_manager: &mut InstanceManager,
    ) -> Result<()> {
        // If the window is not focus, we just focus the instance, but not the view for now.

        let focused_view = {
            if window_focused {
                instance_manager.get_view_by_role(instance, ViewRole::Primary)?
            } else {
                None
            }
        };

        let transitions = self.focus_manager.focus(instance, focused_view);
        Self::transition(transitions, instance_manager)
    }

    fn transition(
        transitions: Vec<FocusTransition>,
        instance_manager: &mut InstanceManager,
    ) -> Result<()> {
        for transition in transitions {
            match transition {
                FocusTransition::UnfocusView(instance, view) => {
                    instance_manager.send_view_event(instance, view, ViewEvent::Focused(false))?;
                }
                FocusTransition::FocusView(instance, view) => {
                    instance_manager.send_view_event(instance, view, ViewEvent::Focused(true))?;
                }
                FocusTransition::UnfocusInstance(_instance_id) => {}
                FocusTransition::FocusInstance(_instance_id) => {}
            }
        }
        Ok(())
    }
}
