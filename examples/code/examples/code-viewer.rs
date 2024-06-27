use std::sync::{Arc, Mutex};

use anyhow::Result;
use cosmic_text::{fontdb, FontSystem};
use tracing::info;
use winit::dpi::LogicalSize;

use massive_geometry::{Camera, SizeI};
use massive_scene::PositionedShape;
use massive_shell::{shell, ApplicationContext};
use shared::{
    application::{Application, UpdateResponse},
    code_viewer::{self, AttributedCode},
};

const CANVAS_ID: &str = "massive-code";

fn main() -> Result<()> {
    shared::main(async_main)
}

async fn async_main() -> Result<()> {
    shell::run(code_viewer).await
}

async fn code_viewer(mut ctx: ApplicationContext) -> Result<()> {
    // let env_filter = EnvFilter::from_default_env();
    // let console_formatter = tracing_subscriber::fmt::Layer::default();
    // // let (flame_layer, _flame_guard) = FlameLayer::with_file("./tracing.folded").unwrap();

    // let now: DateTime<Local> = Local::now();
    // #[allow(unused)]
    // let time_code = now.format("%Y%m%d%H%M").to_string();

    // // let (chrome_layer, _chrome_guard) = tracing_chrome::ChromeLayerBuilder::new()
    // //     .file(format!("./{time_code}-massive-trace.json"))
    // //     .build();

    // Registry::default()
    //     // Filter seems to be applied globally, which is what we want.
    //     .with(env_filter)
    //     // Console formatter currently captures only log::xxx! macros for some reason.
    //     .with(console_formatter)
    //     // .with(flame_layer)
    //     // .with(chrome_layer)
    //     .init();

    // FontSystem

    let mut font_system = {
        let mut db = fontdb::Database::new();
        db.load_font_data(shared::fonts::JETBRAINS_MONO.to_vec());
        // Use an invariant locale.
        FontSystem::new_with_locale_and_db("en-US".into(), db)
    };

    println!("Loaded {} font faces.", font_system.db().faces().count());

    // Load code.

    // let code: AttributedCode =
    //     serde_json::from_str(&fs::read_to_string("/tmp/code.json").unwrap()).unwrap();
    let code: AttributedCode = postcard::from_bytes(include_bytes!("code.postcard")).unwrap();

    // Shape and layout text.

    let font_size = 32.;
    let line_height = 40.;
    // let font_size = 16.;
    // let line_height = 20.;

    let (glyph_runs, height) = code_viewer::shape_text(
        &mut font_system,
        &code.text,
        &code.attributes,
        font_size,
        line_height,
    );

    // Camera

    let camera = {
        let fovy: f64 = 45.0;
        let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();
        Camera::new((0.0, 0.0, camera_distance), (0.0, 0.0, 0.0))
    };

    // Application

    let initial_size = LogicalSize::new(800., 800.);

    let window = ctx.new_window(initial_size, Some(CANVAS_ID))?;
    let (mut renderer, mut director) = window
        .new_renderer(
            Arc::new(Mutex::new(font_system)),
            camera,
            window.inner_size(),
        )
        .await?;

    let mut application = Application::new(SizeI::new(1280, height as u64));
    let mut current_matrix = application.matrix();
    let matrix = director.cast(current_matrix);
    let position = director.cast(matrix.clone().into());

    // Hold the positioned shapes in this context, otherwise they will disappear.
    let _positioned_shapes: Vec<_> = glyph_runs
        .into_iter()
        .map(|run| director.cast(PositionedShape::new(position.clone(), run)))
        .collect();

    director.action()?;

    loop {
        let window_event = ctx.wait_for_event(&mut renderer).await?;

        info!("Window Event: {window_event:?}");

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
