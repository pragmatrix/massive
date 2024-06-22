use std::sync::{Arc, Mutex};

use anyhow::Result;
use cosmic_text::{fontdb, FontSystem};
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Style, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};
use winit::dpi::LogicalSize;

use massive_geometry::{Camera, Color};
use massive_scene::PositionedShape;
use massive_shapes::TextWeight;
use massive_shell::{shell3, ApplicationContext3};
use shared::{
    application2::{Application2, UpdateResponse},
    code_viewer::{self, TextAttribute},
};

const CANVAS_ID: &str = "massive-syntax";

fn main() -> Result<()> {
    shared::main(async_main)
}

async fn async_main() -> Result<()> {
    shell3::run(syntax).await
}

async fn syntax(mut ctx: ApplicationContext3) -> Result<()> {
    // let data = include_str!("rick-and-morty.json");
    let data = include_str!("books.xml");

    // Load these once at the start of your program
    let ps = SyntaxSet::load_defaults_newlines();
    let ts = ThemeSet::load_defaults();

    println!("themes: {:?}", ts.themes.keys());

    let mut text_attributes = Vec::new();

    let font_size = 32.;
    let line_height = 40.;

    let syntax = ps.find_syntax_by_extension("xml").unwrap();
    let mut h = HighlightLines::new(syntax, &ts.themes["InspiredGitHub"]);
    let mut final_text = String::new();
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

    let mut font_system = {
        let mut db = fontdb::Database::new();
        // let font_dir = example_dir.join("JetBrainsMono-2.304/fonts/ttf");
        // db.load_fonts_dir(font_dir);

        db.load_font_data(shared::fonts::JETBRAINS_MONO.to_vec());
        // Use an invariant locale.
        FontSystem::new_with_locale_and_db("en-US".into(), db)
    };

    let camera = {
        let fovy: f64 = 45.0;
        let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();
        Camera::new((0.0, 0.0, camera_distance), (0.0, 0.0, 0.0))
    };

    let (glyph_runs, height) = code_viewer::shape_text(
        &mut font_system,
        &final_text,
        &text_attributes,
        font_size,
        line_height,
    );

    let font_system = Arc::new(Mutex::new(font_system));

    // Window

    let inner_size = LogicalSize::new(800., 800.);
    let window = ctx.new_window(inner_size, Some(CANVAS_ID))?;
    let (mut renderer, mut director) = window
        .new_renderer(font_system, camera, window.inner_size())
        .await?;

    // Application

    let mut application = Application2::new((1280, height as u64));
    let mut current_matrix = application.matrix();
    let matrix = director.cast(current_matrix);

    // Hold the positioned shapes in this context, otherwise they will disappear.
    let _positioned_shapes: Vec<_> = glyph_runs
        .into_iter()
        .map(|run| director.cast(PositionedShape::new(matrix.clone(), run)))
        .collect();

    director.action()?;

    loop {
        let window_event = ctx.wait_for_event(&mut renderer).await?;

        match application.update(window_event) {
            UpdateResponse::Exit => return Ok(()),
            UpdateResponse::Continue => {}
        }

        // DI: This check has to be done in the renderer and the renderer has to decide when it
        // needs to redraw.
        let new_matrix = application.matrix();
        if new_matrix != current_matrix {
            matrix.update(new_matrix);
            current_matrix = new_matrix;
            director.action()?;
        }
    }
}
