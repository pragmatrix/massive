use std::sync::{Arc, Mutex};

use anyhow::Result;
use cosmic_text::{FontSystem, fontdb};
use tracing::info;
use winit::dpi::LogicalSize;

use massive_geometry::{Camera, SizeI};
use massive_scene::Visual;
use massive_shell::{ApplicationContext, Scene, shell};
use shared::{
    application::{Application, UpdateResponse},
    attributed_text::{self, AttributedText},
};

const CANVAS_ID: &str = "massive-code";

#[tokio::main]
async fn main() -> Result<()> {
    shell::run(code_viewer)
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
    let code: AttributedText = postcard::from_bytes(include_bytes!("code.postcard")).unwrap();

    // Shape and layout text.

    let font_size = 32.;
    let line_height = 40.;
    // let font_size = 16.;
    // let line_height = 20.;

    let (glyph_runs, height) = attributed_text::shape_text(
        &mut font_system,
        &code.text,
        &code.attributes,
        font_size,
        line_height,
        None,
    );

    // Camera

    let camera = {
        let fovy: f64 = 45.0;
        let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();
        Camera::new((0.0, 0.0, camera_distance), (0.0, 0.0, 0.0))
    };

    // Application

    let initial_size = LogicalSize::new(800., 800.);

    let window = ctx.new_window(initial_size, Some(CANVAS_ID)).await?;
    // Using inner size screws up the renderer initialization, because the window is not sized yet.
    // So we compute the proper physical for now.
    let physical_size = initial_size.to_physical(window.scale_factor());
    let scene = Scene::default();
    let mut renderer = window
        .new_renderer(Arc::new(Mutex::new(font_system)), camera, physical_size)
        .await?;

    let page_size = SizeI::new(1280, height as u64);
    let mut application = Application::default();
    let matrix = scene.stage(application.matrix(page_size));
    let location = scene.stage(matrix.clone().into());

    // Hold the visual in this context, otherwise it will disappear.
    let _visual = scene.stage(Visual::new(
        location.clone(),
        glyph_runs.into_iter().map(|m| m.into()).collect::<Vec<_>>(),
    ));

    loop {
        let event = ctx.wait_for_shell_event(&mut renderer).await?;
        let _cycle = scene.begin_update_cycle(&mut renderer, Some(&event))?;

        info!("Event: {event:?}");

        if let Some(window_event) = event.window_event_for_id(window.id()) {
            match application.update(window_event) {
                UpdateResponse::Exit => return Ok(()),
                UpdateResponse::Continue => {}
            }
        }

        matrix.update_if_changed(application.matrix(page_size));
    }
}
