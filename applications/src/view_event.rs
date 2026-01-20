use std::path::PathBuf;

use winit::event::{self, DeviceId, WindowEvent};

use massive_geometry::SizePx;
use massive_input::{AggregationEvent, InputEvent};

use crate::ViewId;

/// The events a view can receive.
///
/// Most of them are taken from winit::WindowEvent and simplified if appropriate.
#[derive(Debug, Clone)]
pub enum ViewEvent {
    Resized(SizePx),
    CloseRequested,
    DroppedFile(PathBuf),
    HoveredFile(PathBuf),
    HoveredFileCancelled,
    /// Feature: This is probably related to a "level of detail" management.
    Focused(bool),
    KeyboardInput {
        device_id: event::DeviceId,
        event: event::KeyEvent,
        is_synthetic: bool,
    },
    /// Ergonomics: Document when this is sent (only when Focused?), otherwise, an explicit query
    /// needs to be made.
    ModifiersChanged(event::Modifiers),
    Ime(event::Ime),
    CursorMoved {
        device_id: event::DeviceId,
        /// (x,y) coords in pixels relative to the top-left corner of the view. Because the range
        /// of this data is limited by the display area and it may have been transformed by
        /// the OS to implement effects such as cursor acceleration, it should not be used
        /// to implement non-cursor-like interactions such as 3D camera control.
        position: (f64, f64),
    },
    // Naming: Should probably be renamed to PointerEntered / PointerLeft?
    CursorEntered {
        device_id: event::DeviceId,
    },
    CursorLeft {
        device_id: event::DeviceId,
    },
    MouseWheel {
        device_id: event::DeviceId,
        delta: event::MouseScrollDelta,
        phase: event::TouchPhase,
    },
    MouseInput {
        device_id: event::DeviceId,
        state: event::ElementState,
        button: event::MouseButton,
    },
    // Feature: PinchGesture, PanGesture, DoubleTapGesture, RotationGesture, TouchpadPressure,
    // AxisMotion, Touch

    // Detail: ScaleFactorChanged may not be needed. If it happens, the instance manager should take
    // care of it.
}

impl ViewEvent {
    pub fn from_window_event(window_event: &WindowEvent) -> Option<Self> {
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
            WindowEvent::CursorMoved {
                device_id,
                position,
            } => Some(ViewEvent::CursorMoved {
                device_id: *device_id,
                position: (position.x, position.y),
            }),
            WindowEvent::Focused(focused) => Some(ViewEvent::Focused(*focused)),
            WindowEvent::Resized(size) => {
                Some(ViewEvent::Resized((size.width, size.height).into()))
            }

            // Unhandled events
            WindowEvent::ActivationTokenDone { .. } => None,
            WindowEvent::Moved(..) => None,
            WindowEvent::Destroyed => None,
            WindowEvent::PinchGesture { .. } => None,
            WindowEvent::PanGesture { .. } => None,
            WindowEvent::DoubleTapGesture { .. } => None,
            WindowEvent::RotationGesture { .. } => None,
            WindowEvent::TouchpadPressure { .. } => None,
            WindowEvent::AxisMotion { .. } => None,
            WindowEvent::Touch(..) => None,
            WindowEvent::ScaleFactorChanged { .. } => None,
            WindowEvent::ThemeChanged(..) => None,
            WindowEvent::Occluded(..) => None,
            WindowEvent::RedrawRequested => None,
        }
    }
}

impl InputEvent for ViewEvent {
    type ScopeId = ViewId;

    fn to_aggregation_event(&self) -> Option<AggregationEvent> {
        match self {
            ViewEvent::CursorMoved {
                device_id,
                position,
            } => Some(AggregationEvent::CursorMoved {
                device_id: *device_id,
                position: (*position).into(),
            }),
            ViewEvent::CursorEntered { device_id } => Some(AggregationEvent::CursorEntered {
                device_id: *device_id,
            }),
            ViewEvent::CursorLeft { device_id } => Some(AggregationEvent::CursorLeft {
                device_id: *device_id,
            }),
            ViewEvent::MouseInput {
                device_id,
                state,
                button,
                ..
            } => Some(AggregationEvent::MouseInput {
                device_id: *device_id,
                state: *state,
                button: *button,
            }),
            ViewEvent::ModifiersChanged(modifiers) => {
                Some(AggregationEvent::ModifiersChanged(*modifiers))
            }
            _ => None,
        }
    }

    fn device(&self) -> Option<DeviceId> {
        match self {
            ViewEvent::KeyboardInput { device_id, .. }
            | ViewEvent::CursorMoved { device_id, .. }
            | ViewEvent::CursorEntered { device_id }
            | ViewEvent::CursorLeft { device_id }
            | ViewEvent::MouseWheel { device_id, .. }
            | ViewEvent::MouseInput { device_id, .. } => Some(*device_id),
            _ => None,
        }
    }
}
