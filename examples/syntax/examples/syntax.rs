use std::convert::Infallible;

use anyhow::Result;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use winit::dpi::LogicalSize;

use massive_applications::prelude::*;
use massive_applications::{Application, InstanceContext, InstanceEvent, ViewEvent};
use massive_geometry::{Color, SizePx};
use massive_scene::prelude::*;
use massive_shapes::{FontPolicy, ShapingEngineKind, TextWeight};
use massive_shell::shell;

use shared::attributed_text::{self, TextAttribute};
use shared::fonts::JETBRAINS_MONO;
use shared::gesture_transform::{GestureTransform, UpdateResponse};
use shared::view_center::ViewCenter;

fn main() -> Result<()> {
    shell::run(
        Application::new("Syntax", syntax),
        FontPolicy::bare(ShapingEngineKind::CosmicText),
    )
}

async fn syntax(mut ctx: InstanceContext) -> Result<()> {
    let mut final_text = String::new();
    let mut text_attributes = Vec::new();
    {
        // let data = include_str!("rick-and-morty.json");
        let data = include_str!("books.xml");

        // Load these once at the start of your program
        let ps = SyntaxSet::load_defaults_newlines();
        let ts = ThemeSet::load_defaults();

        println!("themes: {:?}", ts.themes.keys());

        let syntax = ps.find_syntax_by_extension("xml").unwrap();
        let mut h = HighlightLines::new(syntax, &ts.themes["InspiredGitHub"]);
        for line in LinesWithEndings::from(data) {
            let ranges: Vec<(Style, &str)> = h.highlight_line(line, &ps).unwrap();

            for (style, str) in ranges {
                let foreground = style.foreground;
                let attribute = TextAttribute {
                    range: final_text.len()..final_text.len() + str.len(),
                    color: Color::from((foreground.r, foreground.g, foreground.b, foreground.a)),
                    weight: TextWeight::NORMAL,
                };

                if style.font_style != FontStyle::empty() {
                    todo!("Support Font Style");
                }

                final_text.push_str(str);
                text_attributes.push(attribute);
            }
        }
    }

    // The shell built the task's manager from the font policy this application names, so shaping
    // and rendering share one identity world (ADR 0005).
    fonts().load_font(JETBRAINS_MONO)?;

    let font_size = 32.;
    let line_height = 40.;

    // The handle is `!Send` and is dropped at the end of this block, before any await, so the
    // example's future stays `Send` for `Application::new`.
    let (glyph_runs, height) = {
        let mut shaper = shaper();
        attributed_text::shape_text(
            &mut shaper,
            &final_text,
            &text_attributes,
            font_size,
            line_height,
            None,
        )
    };

    // View

    let inner_size = LogicalSize::new(800., 800.).to_physical(ctx.primary_monitor_scale_factor());
    let view_size = SizePx::new(inner_size.width, inner_size.height);
    let mut view = ctx.view(view_size).build()?;
    let mut center = ViewCenter::new(&view, view_size);

    // Content

    let content_size = (1280, height as u32);
    let mut gestures = GestureTransform::default();
    let mut transform = gestures.get_transform(content_size).submit();
    let position = transform
        .to_location()
        .relative_to(center.location())
        .submit();

    // Hold the submitted visual, otherwise it will disappear.
    let _visual = glyph_runs
        .into_iter()
        .map(|run| run.into())
        .collect::<Vec<_>>()
        .at(&position)
        .submit();

    loop {
        transform.update_if_changed(gestures.get_transform(content_size));
        ctx.submit(begin_frame().submission())?;

        for event in ctx.wait_for_events::<Infallible>().await? {
            match event {
                InstanceEvent::View(_, view_event) => {
                    if let ViewEvent::Resized(size) = view_event {
                        center.resize(&mut view, size);
                    }
                    match gestures.update(&view_event) {
                        UpdateResponse::Exit => return Ok(()),
                        UpdateResponse::Continue => {}
                    }
                }
                InstanceEvent::Shutdown(_) => return Ok(()),
                InstanceEvent::ApplyAnimations(_)
                | InstanceEvent::FullscreenRequested
                | InstanceEvent::Tool(_) => {}
                InstanceEvent::Custom(event) => match event {},
            }
        }
    }
}
