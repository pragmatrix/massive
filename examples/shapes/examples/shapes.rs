use anyhow::Result;
use winit::dpi::LogicalSize;

use massive_geometry::{Color, Rect, Size};
use massive_scene::Visual;
use massive_shapes::{
    ChamferRect, Circle, Ellipse, Rect as FilledRect, RoundRect, Shape, StrokeRect,
};
use massive_shell::{Scene, ShellContext, shell};
use shared::application::{Application, UpdateResponse};

#[tokio::main]
async fn main() -> Result<()> {
    shell::run(run)
}

async fn run(mut ctx: ShellContext) -> Result<()> {
    // Window
    let window_size = LogicalSize::new(1024.0, 768.0);
    let window = ctx.new_window(window_size).await?;

    let mut renderer = window.renderer().with_shapes().build().await?;

    // Scene & application
    let mut application = Application::default();
    let scene = ctx.new_scene();

    // Logical page size based on layout of shapes (we adjust after computing bounds)
    // We'll build shapes around (0,0) then center them by translating the matrix to the page center.

    let mut shapes: Vec<Shape> = vec![
        Shape::Rect(FilledRect {
            rect: Rect::new((0.0, 0.0), Size::new(200.0, 120.0)),
            color: Color::from((0.9, 0.2, 0.2, 1.0)),
        }),
        Shape::RoundRect(RoundRect {
            rect: Rect::new((240.0, 0.0), Size::new(200.0, 120.0)),
            corner_radius: 24.0,
            color: Color::from((0.2, 0.7, 0.3, 1.0)),
        }),
        Shape::Circle(Circle {
            rect: Rect::new((480.0, 0.0), Size::new(120.0, 120.0)),
            color: Color::from((0.2, 0.3, 0.9, 1.0)),
        }),
        Shape::StrokeRect(StrokeRect {
            rect: Rect::new((640.0, 0.0), Size::new(200.0, 120.0)),
            stroke: Size::new(6.0, 6.0),
            color: Color::from((0.95, 0.75, 0.2, 1.0)),
        }),
        Shape::Ellipse(Ellipse {
            rect: Rect::new((880.0, 0.0), Size::new(180.0, 120.0)),
            color: Color::from((0.4, 0.85, 0.4, 1.0)),
        }),
        Shape::ChamferRect(ChamferRect {
            rect: Rect::new((1080.0, 0.0), Size::new(200.0, 120.0)),
            chamfer: 12.0,
            color: Color::from((0.85, 0.4, 0.7, 1.0)),
        }),
        // Overlapping translucent stack
        Shape::Rect(FilledRect {
            rect: Rect::new((0.0, 180.0), Size::new(240.0, 160.0)),
            color: Color::from((0.2, 0.9, 0.9, 0.6)),
        }),
        Shape::Circle(Circle {
            rect: Rect::new((80.0, 220.0), Size::new(160.0, 160.0)),
            color: Color::from((0.9, 0.2, 0.9, 0.6)),
        }),
        Shape::RoundRect(RoundRect {
            rect: Rect::new((160.0, 260.0), Size::new(180.0, 160.0)),
            corner_radius: 32.0,
            color: Color::from((0.9, 0.6, 0.2, 0.6)),
        }),
        Shape::Ellipse(Ellipse {
            rect: Rect::new((280.0, 210.0), Size::new(200.0, 120.0)),
            color: Color::from((0.2, 0.5, 0.85, 0.5)),
        }),
    ];

    // Grid of small circles (to the right of the overlapping shapes)
    for y in 0..5 {
        for x in 0..10 {
            let px = 560.0 + x as f64 * 28.0;
            let py = 180.0 + y as f64 * 28.0;
            shapes.push(Shape::Circle(Circle {
                rect: Rect::new((px, py), Size::new(20.0, 20.0)),
                color: Color::from((0.3, 0.3 + x as f32 * 0.05, 0.5 + y as f32 * 0.08, 0.85)),
            }));
        }
    }

    // Compute bounding box of all shapes to derive page size and center transform.
    use massive_geometry::Rect as GeoRect;
    let mut bounds: Option<GeoRect> = None;
    for s in &shapes {
        let r = match s {
            Shape::Rect(r) => r.rect,
            Shape::RoundRect(r) => r.rect,
            Shape::Circle(c) => c.rect,
            Shape::StrokeRect(r) => r.rect,
            Shape::Ellipse(e) => e.rect,
            Shape::ChamferRect(r) => r.rect,
            Shape::GlyphRun(..) | Shape::Custom(..) => continue,
        };
        bounds = Some(if let Some(b) = bounds { b.joined(r) } else { r });
    }
    let bounds = bounds.unwrap_or_else(|| Rect::new((0.0, 0.0), Size::new(1.0, 1.0)));
    let page_width = bounds.size().width.ceil() as u32;
    let page_height = bounds.size().height.ceil() as u32;

    // Center shapes: we shift by -bounds.origin then let Application matrix re-centers page
    // (Application::matrix already centers by half the page size). So we translate shapes so that
    // their local coordinates start at (0,0) relative to bounds.
    let offset_x = -bounds.left;
    let offset_y = -bounds.top;
    for shape in &mut shapes {
        match shape {
            Shape::Rect(r) => {
                r.rect.left += offset_x;
                r.rect.right += offset_x;
                r.rect.top += offset_y;
                r.rect.bottom += offset_y;
            }
            Shape::RoundRect(r) => {
                r.rect.left += offset_x;
                r.rect.right += offset_x;
                r.rect.top += offset_y;
                r.rect.bottom += offset_y;
            }
            Shape::Circle(c) => {
                c.rect.left += offset_x;
                c.rect.right += offset_x;
                c.rect.top += offset_y;
                c.rect.bottom += offset_y;
            }
            Shape::StrokeRect(r) => {
                r.rect.left += offset_x;
                r.rect.right += offset_x;
                r.rect.top += offset_y;
                r.rect.bottom += offset_y;
            }
            Shape::Ellipse(e) => {
                e.rect.left += offset_x;
                e.rect.right += offset_x;
                e.rect.top += offset_y;
                e.rect.bottom += offset_y;
            }
            Shape::ChamferRect(r) => {
                r.rect.left += offset_x;
                r.rect.right += offset_x;
                r.rect.top += offset_y;
                r.rect.bottom += offset_y;
            }
            Shape::GlyphRun(_) | Shape::Custom(..) => {}
        }
    }

    let matrix = scene.stage(application.matrix((page_width, page_height)));
    let location = scene.stage(matrix.clone().into());

    let _visual = scene.stage(Visual::new(location.clone(), shapes));

    loop {
        let event = ctx.wait_for_shell_event().await?;

        if let Some(window_event) = event.window_event_for_id(window.id()) {
            match application.update(window_event) {
                UpdateResponse::Exit => return Ok(()),
                UpdateResponse::Continue => {}
            }
            matrix.update_if_changed(application.matrix((page_width, page_height)));
        }
        scene.render_to(&mut renderer, Some(event))?;
    }
}
