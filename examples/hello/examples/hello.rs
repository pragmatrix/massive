use std::{
    rc::Rc,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use cosmic_text as text;
use massive_scene::legacy;
use shared::positioning;
use text::FontSystem;
use winit::{
    dpi::LogicalSize,
    event::{KeyEvent, WindowEvent},
    keyboard::{Key, NamedKey},
};

use massive_geometry::{Camera, Color, Identity, Matrix4, Vector3};
use massive_shapes::{GlyphRun, GlyphRunMetrics, GlyphRunShape, Shape, TextWeight};
use massive_shell::{shell, ApplicationContext};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    shell::run(application).await
}

async fn application(mut ctx: ApplicationContext) -> Result<()> {
    let font_system = Arc::new(Mutex::new(FontSystem::new()));

    let fovy: f64 = 45.0;
    let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();
    let mut camera = Camera::new((0.0, 0.0, camera_distance), (0.0, 0.0, 0.0));

    // camera.eye = Point3::new(0.8999999999999992, 0.0, 0.11421356237309382);

    let hello_world = "Hello, world!";
    let shapes = render(&mut font_system.lock().unwrap(), hello_world);

    let window = ctx.new_window(LogicalSize::new(1280, 800), None)?;

    let (mut renderer, mut director) = window
        .new_renderer(font_system, camera, window.inner_size())
        .await?;

    let _positioned_shapes = legacy::into_positioned_shapes(&mut director, shapes);
    director.action()?;

    loop {
        match ctx.wait_for_event(&mut renderer).await? {
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: winit::event::ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                match logical_key {
                    Key::Named(NamedKey::ArrowLeft) => camera.eye += Vector3::new(0.1, 0.0, 0.0),
                    Key::Named(NamedKey::ArrowRight) => camera.eye -= Vector3::new(0.1, 0.0, 0.0),
                    Key::Named(NamedKey::ArrowUp) => camera.eye += Vector3::new(0.0, 0.0, 0.1),
                    Key::Named(NamedKey::ArrowDown) => camera.eye -= Vector3::new(0.0, 0.0, 0.1),
                    _ => {}
                }

                renderer.update_camera(camera);
                println!("eye: {:?}", camera.eye)
            }
            WindowEvent::CloseRequested => {
                return Ok(());
            }
            _ => (),
        }
    }
}

fn render(font_system: &mut FontSystem, str: &str) -> Vec<Shape> {
    const FONT_SIZE: f32 = 100.0;

    let mut glyph_run = shape_text(font_system, str, FONT_SIZE);

    let center_x: i32 = (glyph_run.metrics.width / 2) as _;
    let center_y: i32 = ((glyph_run.metrics.size()).1 / 2) as _;
    let center_translation = Vector3::new(-center_x as f64, -center_y as f64, 0.0);

    glyph_run.translation = center_translation;

    let shapes = vec![GlyphRunShape {
        model_matrix: Rc::new(Matrix4::identity()),
        run: glyph_run,
    }
    .into()];

    shapes
}

fn shape_text(font_system: &mut text::FontSystem, text: &str, font_size: f32) -> GlyphRun {
    let mut buffer = text::BufferLine::new(
        text,
        text::AttrsList::new(text::Attrs::new()),
        text::Shaping::Advanced,
    );
    let line = &buffer.layout(font_system, font_size, f32::MAX, text::Wrap::None, None)[0];
    let placed = positioning::position_glyphs(&line.glyphs);
    let metrics = GlyphRunMetrics {
        max_ascent: line.max_ascent as u32,
        max_descent: line.max_descent as u32,
        width: line.w.ceil() as u32,
    };

    GlyphRun::new(
        (0.0, 0.0, 0.0),
        metrics,
        Color::BLACK,
        TextWeight::NORMAL,
        placed,
    )
}
