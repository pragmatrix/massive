use std::collections::HashMap;

use massive_geometry::{Matrix4, SizeI, VectorI};
use winit::{
    event::{
        DeviceId, ElementState, KeyEvent, Modifiers, MouseButton, MouseScrollDelta, TouchPhase,
        WindowEvent,
    },
    keyboard::{Key, NamedKey},
};

enum ActiveGesture {
    Movement(MovementGesture),
    Rotation(RotationGesture),
}

#[derive(Default)]
pub struct Application {
    gesture: Option<ActiveGesture>,

    /// Tracked positions of all devices.
    positions: HashMap<DeviceId, VectorI>,
    modifiers: Modifiers,

    /// Current x / y Translation.
    translation: VectorI,
    translation_z: i32,
    /// Rotation in discrete degrees.
    rotation: VectorI,
}

struct MovementGesture {
    origin: VectorI,
    translation_origin: VectorI,
}

struct RotationGesture {
    origin: VectorI,
    rotation_origin: VectorI,
}

const MOUSE_WHEEL_PIXEL_DELTA_TO_Z_PIXELS: f64 = 0.25;
const MOUSE_WHEEL_LINE_DELTA_TO_Z_PIXELS: i32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateResponse {
    Continue,
    Exit,
}

impl Application {
    #[must_use]
    pub fn update(&mut self, window_event: &WindowEvent) -> UpdateResponse {
        match window_event {
            // Forward to application for more control?
            WindowEvent::CloseRequested
            | WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        state: ElementState::Pressed,
                        logical_key: Key::Named(NamedKey::Escape),
                        ..
                    },
                ..
            } => return UpdateResponse::Exit,
            WindowEvent::CursorMoved {
                device_id,
                position,
            } => {
                // Track positions.
                //
                // These positions aren't discrete / integral on macOS, but why?
                let current = VectorI::new(position.x.round() as _, position.y.round() as _);
                self.positions.insert(*device_id, current);

                // Is there an ongoing movement on the left mouse button?
                if let Some(gesture) = &self.gesture {
                    match gesture {
                        ActiveGesture::Movement(movement) => {
                            let delta = current - movement.origin;
                            self.translation = movement.translation_origin + delta;
                        }
                        ActiveGesture::Rotation(rotation) => {
                            let delta = current - rotation.origin;
                            self.rotation = rotation.rotation_origin + delta;
                        }
                    }
                }
            }
            WindowEvent::MouseWheel {
                delta: MouseScrollDelta::PixelDelta(physical_position),
                phase: TouchPhase::Moved,
                ..
            } => {
                self.translation_z +=
                    (physical_position.y * MOUSE_WHEEL_PIXEL_DELTA_TO_Z_PIXELS).round() as i32
            }
            WindowEvent::MouseWheel {
                delta: MouseScrollDelta::LineDelta(_, y_delta),
                phase: TouchPhase::Moved,
                ..
            } => self.translation_z += y_delta.round() as i32 * MOUSE_WHEEL_LINE_DELTA_TO_Z_PIXELS,
            WindowEvent::MouseInput {
                device_id,
                state,
                button: MouseButton::Left,
            } if self.positions.contains_key(device_id) => {
                if state.is_pressed() {
                    if self.modifiers.state().super_key() {
                        self.gesture = Some(ActiveGesture::Rotation(RotationGesture {
                            origin: self.positions[device_id],
                            rotation_origin: self.rotation,
                        }));
                    } else {
                        self.gesture = Some(ActiveGesture::Movement(MovementGesture {
                            origin: self.positions[device_id],
                            translation_origin: self.translation,
                        }));
                    }
                } else {
                    self.gesture = None;
                }
            }
            WindowEvent::MouseInput {
                device_id,
                state,
                button: MouseButton::Middle,
            } => {
                if state.is_pressed() {
                    self.gesture = Some(ActiveGesture::Rotation(RotationGesture {
                        origin: self.positions[device_id],
                        rotation_origin: self.rotation,
                    }));
                } else {
                    self.gesture = None;
                }
            }

            WindowEvent::MouseInput {
                button: MouseButton::Right,
                ..
            } => {
                self.rotation = VectorI::default();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                if self.modifiers != *modifiers {
                    // If there is an ongoing move and modifiers change, reset origins.
                    // if let Some(ref mut mouse_pressed) = self.left_mouse_button_pressed {
                    //     mouse_pressed.origin = self.positions[&mouse_pressed.device_id];
                    //     mouse_pressed.translation_origin = self.translation;
                    //     mouse_pressed.rotation_origin = self.rotation;
                    // }

                    self.modifiers = *modifiers
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: _,
                        state: _,
                        ..
                    },
                ..
            } => {
                // if state == winit::event::ElementState::Pressed {
                //     match logical_key {
                //         Key::Named(NamedKey::ArrowLeft) => {
                //             self.camera.eye += Vector3::new(0.1, 0.0, 0.0)
                //         }
                //         Key::Named(NamedKey::ArrowRight) => {
                //             self.camera.eye -= Vector3::new(0.1, 0.0, 0.0)
                //         }
                //         Key::Named(NamedKey::ArrowUp) => {
                //             self.camera.eye += Vector3::new(0.0, 0.0, 0.1)
                //         }
                //         Key::Named(NamedKey::ArrowDown) => {
                //             self.camera.eye -= Vector3::new(0.0, 0.0, 0.1)
                //         }
                //         _ => {}
                //     }
                // } else {
                //     {}
                // }
            }
            _ => (),
        }

        UpdateResponse::Continue
    }

    pub fn matrix(&self, page_size: impl Into<SizeI>) -> Matrix4 {
        let page_size = page_size.into();

        let page_x_center: f64 = -((page_size.x / 2) as f64);
        let page_y_center: f64 = -((page_size.y / 2) as f64);
        let center_transformation =
            Matrix4::from_translation((page_x_center, page_y_center, 0.0).into());
        let current_translation = Matrix4::from_translation(
            (
                self.translation.x as _,
                self.translation.y as _,
                self.translation_z as _,
            )
                .into(),
        );
        let angle_x = (self.rotation.x as f64 / 10.).to_radians();
        let angle_y = (-self.rotation.y as f64 / 10.).to_radians();

        let x_rotation = Matrix4::from_rotation_y(angle_x);
        let y_rotation = Matrix4::from_rotation_x(angle_y);

        current_translation * y_rotation * x_rotation * center_transformation
    }
}
