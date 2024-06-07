use std::{collections::HashMap, rc::Rc};

use anyhow::Result;
use massive_geometry::{Camera, Matrix4, PointI, SizeI, Vector3};
use massive_shapes::{GlyphRun, GlyphRunShape, Shape};
use shell::Shell;
use winit::{
    event::{
        DeviceId, KeyEvent, Modifiers, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
    },
    event_loop::EventLoop,
    keyboard::{Key, NamedKey},
    window::{Window, WindowBuilder},
};

use massive_shell as shell;

enum ActiveGesture {
    Movement(MovementGesture),
    Rotation(RotationGesture),
}

pub struct Application2 {
    page_size: SizeI,

    gesture: Option<ActiveGesture>,

    /// Tracked positions of all devices.
    positions: HashMap<DeviceId, PointI>,
    modifiers: Modifiers,

    /// Current x / y Translation.
    translation: PointI,
    translation_z: i32,
    /// Rotation in discrete degrees.
    rotation: PointI,
}

impl Application2 {
    pub fn new(page_size: impl Into<SizeI>) -> Self {
        Self {
            page_size: page_size.into(),
            gesture: None,
            positions: HashMap::new(),
            modifiers: Modifiers::default(),
            translation: PointI::default(),
            translation_z: 0,
            rotation: PointI::default(),
        }
    }
}

struct MovementGesture {
    origin: PointI,
    translation_origin: PointI,
}

struct RotationGesture {
    origin: PointI,
    rotation_origin: PointI,
}

const MOUSE_WHEEL_PIXEL_DELTA_TO_Z_PIXELS: f64 = 0.25;
const MOUSE_WHEEL_LINE_DELTA_TO_Z_PIXELS: i32 = 16;

impl Application2 {
    pub fn update(&mut self, window_event: WindowEvent) {
        match window_event {
            WindowEvent::CursorMoved {
                device_id,
                position,
            } => {
                // Track positions.
                //
                // These positions aren't discrete / integral on macOS, but why?
                let current = PointI::new(position.x.round() as _, position.y.round() as _);
                self.positions.insert(device_id, current);

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
            } if self.positions.contains_key(&device_id) => {
                if state.is_pressed() {
                    if self.modifiers.state().super_key() {
                        self.gesture = Some(ActiveGesture::Rotation(RotationGesture {
                            origin: self.positions[&device_id],
                            rotation_origin: self.rotation,
                        }));
                    } else {
                        self.gesture = Some(ActiveGesture::Movement(MovementGesture {
                            origin: self.positions[&device_id],
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
                        origin: self.positions[&device_id],
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
                self.rotation = PointI::default();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                if self.modifiers != modifiers {
                    // If there is an ongoing move and modifiers change, reset origins.
                    // if let Some(ref mut mouse_pressed) = self.left_mouse_button_pressed {
                    //     mouse_pressed.origin = self.positions[&mouse_pressed.device_id];
                    //     mouse_pressed.translation_origin = self.translation;
                    //     mouse_pressed.rotation_origin = self.rotation;
                    // }

                    self.modifiers = modifiers
                }
            }
            WindowEvent::KeyboardInput {
                event: KeyEvent {
                    logical_key, state, ..
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
    }

    pub fn matrix(&self) -> Matrix4 {
        // let mut shapes = Vec::new();

        let page_x_center: f64 = -((self.page_size.width / 2) as f64);
        let page_y_center: f64 = -((self.page_size.height / 2) as f64);
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
        let angle_x = cgmath::Rad((self.rotation.x as f64 / 10.).to_radians());
        let angle_y = cgmath::Rad((-self.rotation.y as f64 / 10.).to_radians());

        let x_rotation = Matrix4::from_angle_y(angle_x);
        let y_rotation = Matrix4::from_angle_x(angle_y);

        let current_transformation =
            current_translation * y_rotation * x_rotation * center_transformation;

        // TODO: Move pixel matrix into the renderer and multiply all scene matrices with it.
        // let view_transformation = Rc::new(shell.pixel_matrix() * current_transformation);

        current_transformation

        // for glyph_run in &self.glyph_runs {
        //     // let center_x: i32 = (glyph_run.metrics.width / 2) as _;
        //     // let center_y: i32 = ((glyph_run.metrics.size()).1 / 2) as _;

        //     // TODO: Should we use `Rc` for GlyphRuns, too, so that that the application can keep
        //     // them stored?
        //     shapes.push(
        //         GlyphRunShape {
        //             model_matrix: view_transformation.clone(),
        //             run: glyph_run.clone(),
        //         }
        //         .into(),
        //     );
        // }

//        shapes
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn create_window(event_loop: &EventLoop<()>, _canvas_id: Option<&str>) -> Result<Window> {
    Ok(WindowBuilder::new().build(event_loop)?)
}

// Explicitly query for the canvas, and initialize the window with it.
//
// If we use the implicit of `data-raw-handle="1"`, no resize event will be sent.
#[cfg(target_arch = "wasm32")]
pub fn create_window(event_loop: &EventLoop<()>, canvas_id: Option<&str>) -> Result<Window> {
    use wasm_bindgen::JsCast;
    use winit::platform::web::WindowBuilderExtWebSys;

    let canvas_id = canvas_id.expect("Canvas Id is needed for wasm targets");

    let canvas = web_sys::window()
        .expect("No Window")
        .document()
        .expect("No document")
        .query_selector(&format!("#{canvas_id}"))
        // what a shit-show here, why is the error not compatible with anyhow.
        .map_err(|err| anyhow::anyhow!(err.as_string().unwrap()))?
        .expect("No Canvas with a matching id found");

    let canvas: web_sys::HtmlCanvasElement = canvas
        .dyn_into()
        .map_err(|_| anyhow::anyhow!("Failed to cast to HtmlCanvasElement"))?;

    Ok(WindowBuilder::new()
        .with_canvas(Some(canvas))
        .build(event_loop)?)
}
