use std::sync::{Arc, Mutex};

use anyhow::Result;
use cosmic_text::{FontSystem, fontdb};
use winit::dpi::LogicalSize;

use massive_geometry::{Camera, Color, Rect, Size};
use massive_scene::{Scene, Visual};
use massive_shapes::{Circle, Rect as FilledRect, RoundRect, Shape, StrokeRect};
use massive_shell::{ApplicationContext, shell};
use shared::application::{Application, UpdateResponse};

const CANVAS_ID: &str = "massive-shapes";

#[tokio::main]
async fn main() -> Result<()> { shell::run(run) }

async fn run(mut ctx: ApplicationContext) -> Result<()> {
    // Minimal font system for renderer (text layer currently expects one even if unused).
    let font_system = {
        let mut db = fontdb::Database::new();
        db.load_font_data(shared::fonts::JETBRAINS_MONO.to_vec());
        FontSystem::new_with_locale_and_db("en-US".into(), db)
    };
    let font_system = Arc::new(Mutex::new(font_system));

    // Camera
    let camera = {
        let fovy: f64 = 45.0;
        let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();
        Camera::new((0.0, 0.0, camera_distance), (0.0, 0.0, 0.0))
    };

    // Window
    let window_size = LogicalSize::new(1024.0, 768.0);
    let window = ctx.new_window(window_size, Some(CANVAS_ID)).await?;

    let mut renderer = window
        .new_renderer(font_system, camera, window.inner_size())
        .await?;

    // Scene & application (for simple panning/zooming/rotation like other examples)
    let mut application = Application::default();
    let scene = Scene::new();

    let page_size = (1600u32, 900u32); // virtual canvas area
    let matrix = scene.stage(application.matrix(page_size));
    let location = scene.stage(matrix.clone().into());

    // Create a variety of shapes in pixel space
    let mut shapes: Vec<Shape> = vec![
        // Simple filled rect
        Shape::Rect(FilledRect { rect: Rect::new((50.0, 50.0), Size::new(200.0, 120.0)), color: Color::from((0.9, 0.2, 0.2, 1.0)) }),
        // Rounded rect
        Shape::RoundRect(RoundRect { rect: Rect::new((300.0, 50.0), Size::new(200.0, 120.0)), corner_radius: 24.0, color: Color::from((0.2, 0.7, 0.3, 1.0)) }),
        // Circle (uses bounding rect)
        Shape::Circle(Circle { rect: Rect::new((550.0, 50.0), Size::new(120.0, 120.0)), color: Color::from((0.2, 0.3, 0.9, 1.0)) }),
        // Stroked rect
        Shape::StrokeRect(StrokeRect { rect: Rect::new((700.0, 50.0), Size::new(200.0, 120.0)), stroke: Size::new(6.0, 6.0), color: Color::from((0.95, 0.75, 0.2, 1.0)) }),
        // Overlapping shapes to show alpha blending
        Shape::Rect(FilledRect { rect: Rect::new((100.0, 250.0), Size::new(240.0, 160.0)), color: Color::from((0.2, 0.9, 0.9, 0.6)) }),
        Shape::Circle(Circle { rect: Rect::new((180.0, 290.0), Size::new(160.0, 160.0)), color: Color::from((0.9, 0.2, 0.9, 0.6)) }),
        Shape::RoundRect(RoundRect { rect: Rect::new((260.0, 330.0), Size::new(180.0, 160.0)), corner_radius: 32.0, color: Color::from((0.9, 0.6, 0.2, 0.6)) }),
        // Grid of small circles
    ];

    // Add grid of small circles
    for y in 0..5 { for x in 0..10 { let px = 550.0 + x as f64 * 28.0; let py = 260.0 + y as f64 * 28.0; shapes.push(Shape::Circle(Circle { rect: Rect::new((px, py), Size::new(20.0, 20.0)), color: Color::from((0.3, 0.3 + x as f32 * 0.05, 0.5 + y as f32 * 0.08, 0.85)) })); } }

    // Stage visual so it gets rendered
    let _visual = scene.stage(Visual::new(location.clone(), shapes));

    loop {
        let event = ctx.wait_for_shell_event(&mut renderer).await?;
        let _cycle = ctx.begin_update_cycle(&scene, &mut renderer, Some(&event))?;

        if let Some(window_event) = event.window_event_for_id(window.id()) {
            match application.update(window_event) {
                UpdateResponse::Exit => return Ok(()),
                UpdateResponse::Continue => {}
            }
            matrix.update_if_changed(application.matrix(page_size));
        }
    }
}
