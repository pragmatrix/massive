use std::sync::{Arc, Mutex};

use anyhow::Result;
use cosmic_text::{FontSystem, fontdb};
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Style, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};
use winit::dpi::LogicalSize;

use massive_geometry::{Camera, Color};
use massive_scene::{Scene, Visual};
use massive_shapes::TextWeight;
use massive_shell::{ApplicationContext, shell};
use shared::{
    application::{Application, UpdateResponse},
    attributed_text::{self, TextAttribute},
};

const CANVAS_ID: &str = "massive-syntax";

#[tokio::main]
async fn main() -> Result<()> {
    shell::run(syntax)
}

async fn syntax(mut ctx: ApplicationContext) -> Result<()> {
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

    let font_size = 32.;
    let line_height = 40.;

    let (glyph_runs, height) = attributed_text::shape_text(
        &mut font_system,
        &final_text,
        &text_attributes,
        font_size,
        line_height,
        None,
    );

    let font_system = Arc::new(Mutex::new(font_system));

    // Window

    let inner_size = LogicalSize::new(800., 800.);
    let window = ctx.new_window(inner_size, Some(CANVAS_ID)).await?;
    let scene = Scene::new();
    let mut renderer = window
        .new_renderer(font_system, camera, window.inner_size())
        .await?;

    // Application

    let page_size = (1280, height as u64);
    let mut application = Application::default();
    let matrix = scene.stage(application.matrix(page_size));
    let position = scene.stage(matrix.clone().into());

    // Hold the staged visual, otherwise it will disappear.
    let _visual = scene.stage(Visual::new(
        position.clone(),
        glyph_runs
            .into_iter()
            .map(|run| run.into())
            .collect::<Vec<_>>(),
    ));

    loop {
        let event = ctx.wait_for_shell_event(&mut renderer).await?;
        let _cycle = ctx.begin_update_cycle(&scene, &mut renderer, Some(&event))?;

        if let Some(window_event) = event.window_event_for_id(window.id()) {
            match application.update(window_event) {
                UpdateResponse::Exit => return Ok(()),
                UpdateResponse::Continue => {}
            }
        }

        // DI: This check has to be done in the renderer and the renderer has to decide when it
        // needs to redraw.
        matrix.update_if_changed(application.matrix(page_size));
    }
}
