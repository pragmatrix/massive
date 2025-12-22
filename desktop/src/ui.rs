use std::cmp::Ordering;

use anyhow::Result;
use log::warn;
use massive_animation::{Animated, Interpolation};
use massive_shell::Scene;
use winit::{
    event::{ElementState, WindowEvent},
    keyboard::Key,
};

use massive_applications::{InstanceId, ViewEvent, ViewId, ViewRole};
use massive_geometry::{PixelCamera, Point, Vector3, VectorPx};
use massive_input::Event;
use massive_renderer::RenderGeometry;

use crate::{
    FocusManager, FocusPath, FocusTransition,
    desktop_presenter::DesktopPresenter,
    instance_manager::{InstanceManager, ViewPath},
};

// Architecture: This is all about focus so far. May rename it to DesktopInput or DesktopFocus or
// InputInterface?
//
// Architecture: Every function here needs &InstanceManager.
#[derive(Debug)]
pub struct UI {
    /// The camera
    camera: Animated<PixelCamera>,

    /// The recently touched view with the cursor / mouse, None if it's the desktop background.
    ///
    /// The ids may not exist.
    cursor_focus: Option<CursorFocus>,

    /// This decides to which view and instance the keyboard events are delivered. Basically the
    /// keyboard focus.
    focus_manager: FocusManager,

    /// The current state of the window.
    ///
    /// This is used to remember the previously focused view, because we do unfocus the view.
    /// Architecture: May be the focus-manager should do that.
    window_focus_state: WindowFocusState,
}

#[derive(Debug)]
enum WindowFocusState {
    Unfocused { focused_previously: Option<ViewId> },
    Focused,
}

#[derive(Debug)]
struct CursorFocus {
    path: ViewPath,
    hit_on_view: Vector3,
}

impl UI {
    // Detail: We need a primary instance and view to initialize the UI for now.
    //
    // Detail: This function assumes that the window is focused right now.
    pub fn new(
        view_path: ViewPath,
        instance_manager: &InstanceManager,
        presenter: &DesktopPresenter,
        scene: &Scene,
    ) -> Result<Self> {
        let mut focus_manager = FocusManager::new();
        let focus_transitions = focus_manager.focus(view_path);
        forward_focus_transitions(focus_transitions, instance_manager)?;

        let camera = camera(&focus_manager, presenter).expect("Internal error: No initial focus");

        Ok(Self {
            camera: scene.animated(camera),
            cursor_focus: None,
            focus_manager,
            window_focus_state: WindowFocusState::Focused,
        })
    }

    pub fn focused_instance(&self) -> Option<InstanceId> {
        self.focus_manager.focused_instance()
    }

    pub fn camera(&self) -> PixelCamera {
        self.camera.value()
    }

    pub fn make_foreground(
        &mut self,
        instance: InstanceId,
        instance_manager: &InstanceManager,
        presenter: &DesktopPresenter,
    ) -> Result<()> {
        // If the window is not focus, we just focus the instance.
        let mut focused_view = instance_manager.get_view_by_role(instance, ViewRole::Primary)?;

        // If the window state is unfocused, we don't want to focus the primary view but want it to
        // focus when window focus comes back.
        if let WindowFocusState::Unfocused { focused_previously } = &mut self.window_focus_state {
            *focused_previously = focused_view.take();
        };

        set_focus(
            &mut self.focus_manager,
            (instance, focused_view),
            instance_manager,
        )?;

        let camera = camera(&self.focus_manager, presenter);
        if let Some(camera) = camera {
            self.camera.animate_if_changed(
                camera,
                DesktopPresenter::INSTANCE_TRANSITION_DURATION,
                Interpolation::CubicOut,
            );
        }

        Ok(())
    }

    pub fn handle_input_event(
        &mut self,
        input_event: &Event<WindowEvent>,
        instance_manager: &InstanceManager,
        render_geometry: &RenderGeometry,
    ) -> Result<UiCommand> {
        let send_to_view = |view, event| instance_manager.send_view_event(view, event);

        let send_window_event = |view, window_event| {
            if let Some(view_event) = convert_window_event_to_view_event(window_event) {
                send_to_view(view, view_event)
            } else {
                Ok(())
            }
        };

        let window_event = input_event.event();

        match window_event {
            WindowEvent::Focused(window_focused) => {
                self.set_window_focus(*window_focused, instance_manager)?;
            }

            WindowEvent::CursorMoved { device_id, .. } => {
                let any_pressed = input_event
                    .pointing_device_state(*device_id)
                    .map(|d| d.any_button_pressed())
                    .unwrap_or(false);

                let screen_pos = input_event
                    .pos()
                    .expect("Internal error, a CursorMoved event must support a position");

                // Change the cursor focus only if there is no button pressed.
                //
                // Robustness: There might be a change of the device here.
                if !any_pressed {
                    let hit_result =
                        hit_test_at_point(screen_pos, instance_manager, render_geometry);
                    self.cursor_focus = hit_result.map(|(view, point)| CursorFocus {
                        path: view,
                        hit_on_view: point,
                    });
                } else {
                    // Button is pressed, may update pos only.
                    if let Some(cursor_focus) = &mut self.cursor_focus {
                        if let Some(hit_result) = hit_test_on_view(
                            screen_pos,
                            instance_manager,
                            render_geometry,
                            cursor_focus.path,
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
                    path: view,
                    hit_on_view,
                    ..
                }) = self.cursor_focus
                {
                    send_to_view(
                        view,
                        ViewEvent::CursorMoved {
                            device_id: *device_id,
                            position: (hit_on_view.x, hit_on_view.y),
                        },
                    )?
                }
            }

            // Forwarded to the view with cursor focus
            WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::DroppedFile(_)
            | WindowEvent::HoveredFile(_) => {
                if let Some(CursorFocus { path, .. }) = self.cursor_focus {
                    // Does this event cause a focusing of the view at the current cursor pos?
                    if causes_focus(window_event) && self.focus_manager.focused_view() != Some(path)
                    {
                        set_focus(&mut self.focus_manager, path, instance_manager)?;
                        // Don't forward the event if the focus get changed, but tell the client that it should make the instance the foreground.
                        return Ok(UiCommand::MakeForeground {
                            instance: path.instance,
                        });
                    }

                    send_window_event(path, window_event)?;
                }
            }

            // Keyboard focus
            WindowEvent::KeyboardInput {
                event: key_event, ..
            } => {
                let focused_view = self.focus_manager.focused_view();

                if key_event.state == ElementState::Pressed
                    && !key_event.repeat
                    && input_event.states().is_command()
                    && let Some(ViewPath { instance, .. }) = focused_view
                {
                    match &key_event.logical_key {
                        Key::Character(c) if c.as_str() == "t" => {
                            let application = instance_manager.get_application_name(instance)?;
                            return Ok(UiCommand::StartInstance {
                                application: application.to_string(),
                                originating_instance: instance,
                            });
                        }
                        Key::Character(c) if c.as_str() == "w" => {
                            return Ok(UiCommand::StopInstance { instance });
                        }
                        _ => {}
                    }
                }

                if let Some(view) = focused_view {
                    send_window_event(view, window_event)?;
                }
            }

            WindowEvent::Ime(_) => {
                if let Some(view) = self.focus_manager.focused_view() {
                    send_window_event(view, window_event)?;
                }
            }

            // All views
            WindowEvent::ModifiersChanged(_)
            | WindowEvent::HoveredFileCancelled
            | WindowEvent::CloseRequested => {
                for (view, _view_info) in instance_manager.views() {
                    send_window_event(view, window_event)?;
                }
            }

            WindowEvent::Resized(_) => {}
            _ => {}
        }

        Ok(UiCommand::None)
    }

    fn set_window_focus(
        &mut self,
        focused: bool,
        instance_manager: &InstanceManager,
    ) -> Result<()> {
        match (&self.window_focus_state, focused) {
            (WindowFocusState::Unfocused { focused_previously }, true) => {
                // Refocus if the current instance is the instance of the previous view and it does
                // exist anymore.
                if let Some(view) = *focused_previously
                    && let Some(instance) = self.focus_manager.focused_instance()
                    && instance_manager.exists(instance, Some(view))
                {
                    set_focus(
                        &mut self.focus_manager,
                        (instance, Some(view)),
                        instance_manager,
                    )?;
                }
                self.window_focus_state = WindowFocusState::Focused
            }
            (WindowFocusState::Focused, false) => {
                let focused_view = self.focus_manager.focused_view().map(|p| p.view);
                self.window_focus_state = WindowFocusState::Unfocused {
                    focused_previously: focused_view,
                };
                let transitions = self.focus_manager.unfocus_view();
                forward_focus_transitions(transitions, instance_manager)?;
            }
            _ => {
                warn!("Redundant Window focus change")
            }
        }
        Ok(())
    }
}

/// `true` if the WindowEvent causes focus and should be consumed.
fn causes_focus(e: &WindowEvent) -> bool {
    matches!(
        e,
        WindowEvent::MouseInput {
            state: ElementState::Pressed,
            ..
        }
    )
}

fn set_focus(
    focus_manager: &mut FocusManager,
    path: impl Into<FocusPath>,
    instance_manager: &InstanceManager,
) -> Result<()> {
    let path = path.into();
    assert!(instance_manager.exists(path.instance, path.view));
    let transitions = focus_manager.focus(path);
    forward_focus_transitions(transitions, instance_manager)
}

fn forward_focus_transitions(
    transitions: Vec<FocusTransition>,
    instance_manager: &InstanceManager,
) -> Result<()> {
    for transition in transitions {
        match transition {
            FocusTransition::Exit(FocusPath {
                instance,
                view: Some(view),
            }) => {
                instance_manager.send_view_event((instance, view), ViewEvent::Focused(false))?;
            }
            FocusTransition::Enter(FocusPath {
                instance,
                view: Some(view),
            }) => {
                instance_manager.send_view_event((instance, view), ViewEvent::Focused(true))?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Returns the camera position it should target at.
fn camera(focus_manager: &FocusManager, presenter: &DesktopPresenter) -> Option<PixelCamera> {
    focus_manager
        .focused_instance()
        .and_then(|instance| presenter.instance_transform(instance))
        .map(|target| PixelCamera::look_at(target, PixelCamera::DEFAULT_FOVY))
}

//
// Hit testing
//

fn hit_test_at_point(
    screen_pos: Point,
    instance_manager: &InstanceManager,
    geometry: &RenderGeometry,
) -> Option<(ViewPath, Vector3)> {
    let mut hits = Vec::new();

    for (view, view_info) in instance_manager.views() {
        let location = view_info.location.value();
        let transform = location.transform.value();
        let extents = view_info.extents;

        // Feature: Support parent transforms (and cache?)!
        let matrix = transform.to_matrix4();
        if let Some(local_pos) = geometry.unproject_to_model_z0(screen_pos, &matrix) {
            // Robustness: Are we leaving accuracy on the table here by converting from f64 to i32?
            let v: VectorPx = (local_pos.x as i32, local_pos.y as i32).into();
            if extents.contains(v.to_point()) {
                hits.push((view, local_pos));
            }
        }
    }

    // Sort by z (descending) to get topmost view first
    hits.sort_by(|a, b| b.1.z.partial_cmp(&a.1.z).unwrap_or(Ordering::Equal));

    hits.first().copied()
}

/// Specifically hit tests on a view without considering its boundaries.
///
/// Meaning that the hit test result may be out of bounds.
fn hit_test_on_view(
    screen_pos: Point,
    instance_manager: &InstanceManager,
    render_geometry: &RenderGeometry,
    view: ViewPath,
) -> Option<Vector3> {
    let view_info = instance_manager.get_view(view).ok()?;
    let location = view_info.location.value();
    let matrix = location.transform.value().to_matrix4();
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
        WindowEvent::ModifiersChanged(modifiers) => Some(ViewEvent::ModifiersChanged(*modifiers)),
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
        WindowEvent::Resized(size) => Some(ViewEvent::Resized((size.width, size.height).into())),
        _ => None,
    }
}

#[must_use]
pub enum UiCommand {
    None,
    StartInstance {
        application: String,
        originating_instance: InstanceId,
    },
    MakeForeground {
        instance: InstanceId,
    },
    StopInstance {
        instance: InstanceId,
    },
}
