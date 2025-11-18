use anyhow::Result;
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Style, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};
use winit::dpi::LogicalSize;

use massive_geometry::Color;
use massive_scene::Visual;
use massive_shapes::TextWeight;
use massive_shell::{FontManager, ShellContext, shell};
use shared::{
    application::{Application, UpdateResponse},
    attributed_text::{self, TextAttribute},
};

#[tokio::main]
async fn main() -> Result<()> {
    shell::run(syntax)
}

async fn syntax(mut ctx: ShellContext) -> Result<()> {
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

    let fonts = FontManager::bare("en-US").with_font(shared::fonts::JETBRAINS_MONO);

    let font_size = 32.;
    let line_height = 40.;

    let (glyph_runs, height) = attributed_text::shape_text(
        &mut fonts.lock(),
        &final_text,
        &text_attributes,
        font_size,
        line_height,
        None,
    );

    // Window

    let inner_size = LogicalSize::new(800., 800.);
    let window = ctx.new_window(inner_size).await?;

    let scene = ctx.new_scene();
    let mut renderer = window.renderer().with_text(fonts).build().await?;

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
        let event = ctx.wait_for_shell_event().await?;

        if let Some(window_event) = event.window_event_for_id(window.id()) {
            match application.update(window_event) {
                UpdateResponse::Exit => return Ok(()),
                UpdateResponse::Continue => {}
            }
        }

        // DI: This check has to be done in the renderer and the renderer has to decide when it
        // needs to redraw.
        matrix.update_if_changed(application.matrix(page_size));
        scene.render_to(&mut renderer, Some(event))?;
    }
}
