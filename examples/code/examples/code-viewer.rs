use anyhow::Result;
use tracing::info;
use winit::dpi::LogicalSize;

use massive_geometry::SizePx;
use massive_scene::{At, Object, ToLocation};
use massive_shell::{ApplicationContext, FontManager, shell};
use shared::{
    application::{Application, UpdateResponse},
    attributed_text::{self, AttributedText},
};

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

    let fonts = FontManager::bare("en-US").with_font(shared::fonts::JETBRAINS_MONO);

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
        &mut fonts.lock(),
        &code.text,
        &code.attributes,
        font_size,
        line_height,
        None,
    );

    // Application

    let initial_size = LogicalSize::new(800., 800.).to_physical(ctx.primary_monitor_scale_factor());

    let window = ctx
        .new_window((initial_size.width, initial_size.height))
        .await?;
    // Using inner size screws up the renderer initialization, because the window has no size yet.
    // So we compute the proper physical for now.
    // let physical_size = initial_size.to_physical(window.scale_factor());
    let scene = ctx.new_scene();
    let mut renderer = window.renderer().with_text(fonts).build().await?;

    let content_size = SizePx::new(1280, height as u32);
    let mut application = Application::default();
    let transform = application.get_transform(content_size).enter(&scene);
    let location = transform.to_location().enter(&scene);

    // Hold the visual in this context, otherwise it will disappear.
    let _visual = glyph_runs
        .into_iter()
        .map(|m| m.into())
        .collect::<Vec<_>>()
        .at(&location)
        .enter(&scene);

    loop {
        let event = ctx.wait_for_shell_event().await?;

        info!("Event: {event:?}");

        if let Some(window_event) = event.window_event_for_id(window.id()) {
            match application.update(window_event) {
                UpdateResponse::Exit => return Ok(()),
                UpdateResponse::Continue => {}
            }
        }

        transform.update_if_changed(application.get_transform(content_size));

        renderer.resize_redraw(&event)?;
        scene.render_to(&mut renderer)?;
    }
}
