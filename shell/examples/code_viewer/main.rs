use std::sync::{Arc, Mutex};

use anyhow::Result;
use cosmic_text::{fontdb, FontSystem};
// use hir::db::DefDatabase;
use shared::{
    application,
    code_viewer::{self, AttributedCode},
};
use winit::event_loop::EventLoop;

use massive_geometry::{Camera, SizeI};
use massive_shell::Shell;

const CANVAS_ID: &str = "massive-code";

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<()> {
    env_logger::init();

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    // Use the runtime to block on the async function
    rt.block_on(async_main())
}

#[cfg(target_arch = "wasm32")]
fn main() {
    console_error_panic_hook::set_once();
    console_log::init().expect("Could not initialize logger");

    wasm_bindgen_futures::spawn_local(async {
        match async_main().await {
            Ok(()) => {}
            Err(e) => {
                log::error!("{e}");
            }
        }
    });
}
async fn async_main() -> Result<()> {
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
        db.load_font_data(shared::fonts::jetbrains_mono().to_vec());
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

    // Window

    let event_loop = EventLoop::new()?;
    let window = application::create_window(&event_loop, Some(CANVAS_ID))?;
    let initial_size = window.inner_size();

    // Camera

    let camera = {
        let fovy: f64 = 45.0;
        let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();
        Camera::new((0.0, 0.0, camera_distance), (0.0, 0.0, 0.0))
    };

    // Application

    let application =
        application::Application::new(camera, glyph_runs, SizeI::new(1280, height as u64));

    // Shell

    let font_system = Arc::new(Mutex::new(font_system));
    let mut shell = Shell::new(&window, initial_size, font_system.clone()).await?;
    shell.run(event_loop, &window, application).await
}
