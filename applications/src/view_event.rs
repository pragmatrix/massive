use std::path::PathBuf;

use winit::event::{self, DeviceId, ElementState, KeyEvent, WindowEvent};
use winit::keyboard::Key;

use massive_geometry::{Point, SizePx, Vector};
use massive_input::{AggregationEvent, InputEvent};

/// The events a view can receive.
///
/// Most of them are taken from winit::WindowEvent and simplified if appropriate.
///
/// Because DeviceId is not supported on macOS and iOS, we don't support that for simplicity.
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
        event: event::KeyEvent,
        is_synthetic: bool,
    },
    /// Modifiers are not updated when the target has neither pointer nor keyboard focus, but are
    /// updated as soon it gets either of them back.
    ModifiersChanged(event::Modifiers),
    Ime(event::Ime),
    // This is in view relative coordinates.
    CursorMoved(Point),
    // Naming: Should probably be renamed to PointerEntered / PointerLeft?
    CursorEntered,
    CursorLeft,
    MouseWheel {
        delta: event::MouseScrollDelta,
        phase: event::TouchPhase,
    },
    MouseInput {
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
            WindowEvent::CursorEntered { device_id: _ } => Some(Self::CursorEntered),
            WindowEvent::CursorLeft { device_id: _ } => Some(Self::CursorLeft),
            WindowEvent::MouseInput {
                device_id: _,
                state,
                button,
                ..
            } => Some(ViewEvent::MouseInput {
                state: *state,
                button: *button,
            }),
            WindowEvent::MouseWheel {
                device_id: _,
                delta,
                phase,
            } => Some(ViewEvent::MouseWheel {
                delta: *delta,
                phase: *phase,
            }),
            WindowEvent::ModifiersChanged(modifiers) => {
                Some(ViewEvent::ModifiersChanged(*modifiers))
            }
            WindowEvent::DroppedFile(path) => Some(Self::DroppedFile(path.clone())),
            WindowEvent::HoveredFile(path) => Some(Self::HoveredFile(path.clone())),
            WindowEvent::HoveredFileCancelled => Some(Self::HoveredFileCancelled),
            WindowEvent::CloseRequested => Some(Self::CloseRequested),
            WindowEvent::KeyboardInput {
                device_id: _,
                event,
                is_synthetic,
            } => Some(Self::KeyboardInput {
                event: event.clone(),
                is_synthetic: *is_synthetic,
            }),
            WindowEvent::Ime(ime) => Some(Self::Ime(ime.clone())),
            WindowEvent::CursorMoved {
                device_id: _,
                position,
            } => Some(Self::CursorMoved((position.x, position.y).into())),
            WindowEvent::Focused(focused) => Some(Self::Focused(*focused)),
            WindowEvent::Resized(size) => Some(Self::Resized((size.width, size.height).into())),

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

    /// If this is a keyboard event that indicates that a key was recently pressed
    /// on any keyboard device and is not repeating, returns the key.
    pub fn pressed_key(&self) -> Option<&Key> {
        if let Self::KeyboardInput {
            event:
                KeyEvent {
                    logical_key,
                    state: ElementState::Pressed,
                    repeat: false,
                    ..
                },
            ..
        } = self
        {
            Some(logical_key)
        } else {
            None
        }
    }

    pub fn translate(self, v: Vector) -> ViewEvent {
        match self {
            Self::CursorMoved(position) => Self::CursorMoved(position + v),
            _ => self,
        }
    }
}

impl InputEvent for ViewEvent {
    fn to_aggregation_event(&self) -> Option<AggregationEvent> {
        match self {
            Self::CursorMoved(position) => Some(AggregationEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: *position,
            }),
            Self::CursorEntered => Some(AggregationEvent::CursorEntered {
                device_id: DeviceId::dummy(),
            }),
            Self::CursorLeft => Some(AggregationEvent::CursorLeft {
                device_id: DeviceId::dummy(),
            }),
            Self::MouseInput { state, button, .. } => Some(AggregationEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state: *state,
                button: *button,
            }),
            Self::ModifiersChanged(modifiers) => {
                Some(AggregationEvent::ModifiersChanged(*modifiers))
            }
            _ => None,
        }
    }

    fn device(&self) -> Option<DeviceId> {
        match self {
            ViewEvent::KeyboardInput { .. }
            | ViewEvent::CursorMoved(_)
            | ViewEvent::CursorEntered
            | ViewEvent::CursorLeft
            | ViewEvent::MouseWheel { .. }
            | ViewEvent::MouseInput { .. } => Some(DeviceId::dummy()),
            _ => None,
        }
    }
}
