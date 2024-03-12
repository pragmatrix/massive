use std::rc::Rc;

use cosmic_text as text;
use granularity_geometry::{Camera, Matrix4, Vector3};
use granularity_shapes::{GlyphRun, GlyphRunMetrics, PositionedGlyph, Shape};
use text::CacheKeyFlags;
use winit::{
    event::{KeyEvent, WindowEvent},
    keyboard::{Key, NamedKey},
};

use granularity_shell::{self as shell, Shell};

struct Application {
    camera: Camera,
    hello_world: String,
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let fovy: f64 = 45.0;
    let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();

    let camera = Camera::new((0.0, 0.0, camera_distance), (0.0, 0.0, 0.0));

    // camera.eye = Point3::new(0.8999999999999992, 0.0, 0.11421356237309382);

    let hello_world = "Hello, world!";

    let application = Application {
        camera,
        hello_world: hello_world.to_string(),
    };

    let _ = shell::run(application).await;
}

impl shell::Application for Application {
    fn update(&mut self, window_event: WindowEvent) {
        if let WindowEvent::KeyboardInput {
            event: KeyEvent {
                logical_key, state, ..
            },
            ..
        } = window_event
        {
            if state == winit::event::ElementState::Pressed {
                match logical_key {
                    Key::Named(NamedKey::ArrowLeft) => {
                        self.camera.eye += Vector3::new(0.1, 0.0, 0.0)
                    }
                    Key::Named(NamedKey::ArrowRight) => {
                        self.camera.eye -= Vector3::new(0.1, 0.0, 0.0)
                    }
                    Key::Named(NamedKey::ArrowUp) => self.camera.eye += Vector3::new(0.0, 0.0, 0.1),
                    Key::Named(NamedKey::ArrowDown) => {
                        self.camera.eye -= Vector3::new(0.0, 0.0, 0.1)
                    }
                    _ => {}
                }
            } else {
                {}
            }

            println!("eye: {:?}", self.camera.eye)
        }
    }

    fn render(&self, shell: &mut Shell) -> (Camera, Vec<Shape>) {
        const FONT_SIZE: f32 = 100.0;

        let text = shape_text(&mut shell.font_system, &self.hello_world, FONT_SIZE);

        let shapes = vec![Shape::GlyphRun(
            text.into_glyph_run(Rc::new(shell.pixel_matrix())),
        )];

        (self.camera, shapes)
    }
}

#[derive(Debug)]
pub struct ShapedText {
    pub metrics: GlyphRunMetrics,
    pub glyphs: Vec<PositionedGlyph>,
}

impl ShapedText {
    pub fn new(metrics: GlyphRunMetrics, glyphs: Vec<PositionedGlyph>) -> Self {
        Self { metrics, glyphs }
    }

    pub fn into_glyph_run(self, matrix: Rc<Matrix4>) -> GlyphRun {
        GlyphRun {
            glyphs: self.glyphs,
            model_matrix: matrix.clone(),
            metrics: self.metrics,
        }
    }
}

fn shape_text(font_system: &mut text::FontSystem, text: &str, font_size: f32) -> ShapedText {
    let mut buffer = text::BufferLine::new(
        text,
        text::AttrsList::new(text::Attrs::new()),
        text::Shaping::Advanced,
    );
    let line = &buffer.layout(font_system, font_size, f32::MAX, text::Wrap::None, None)[0];
    let placed = place_glyphs(&line.glyphs);
    let metrics = GlyphRunMetrics {
        max_ascent: line.max_ascent as u32,
        max_descent: line.max_descent as u32,
        width: line.w.ceil() as u32,
    };

    ShapedText::new(metrics, placed)
}

const RENDER_SUBPIXEL: bool = false;

fn place_glyphs(glyphs: &[text::LayoutGlyph]) -> Vec<PositionedGlyph> {
    glyphs
        .iter()
        .map(|glyph| {
            let fractional_pos = if RENDER_SUBPIXEL {
                (glyph.x, glyph.y)
            } else {
                (glyph.x.round(), glyph.y.round())
            };

            let (cc, x, y) = text::CacheKey::new(
                glyph.font_id,
                glyph.glyph_id,
                glyph.font_size,
                fractional_pos,
                CacheKeyFlags::empty(),
            );
            // Note: hitbox with is fractional, but does not change with / without subpixel
            // rendering.
            PositionedGlyph::new(cc, (x, y), glyph.w)
        })
        .collect()
}
