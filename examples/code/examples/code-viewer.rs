use std::convert::Infallible;

use anyhow::Result;
use tracing::info;
use winit::dpi::LogicalSize;

use massive_applications::prelude::*;
use massive_applications::{Application, InstanceContext, InstanceEvent, ViewEvent};
use massive_geometry::SizePx;
use massive_scene::prelude::*;
use massive_shapes::{FontPolicy, ShapingEngineKind};
use massive_shell::shell;

use shared::attributed_text::{self, AttributedText};
use shared::fonts::JETBRAINS_MONO;
use shared::gesture_transform::{GestureTransform, UpdateResponse};
use shared::view_center::ViewCenter;

fn main() -> Result<()> {
    shell::run(
        Application::new("Code Viewer", code_viewer),
        FontPolicy::bare(ShapingEngineKind::CosmicText),
    )
}

async fn code_viewer(mut ctx: InstanceContext) -> Result<()> {
    // spellcheck: ignore
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

    // The shell built the task's manager from the font policy this application names, so shaping
    // and rendering share one identity world (ADR 0005).
    fonts().load_font(JETBRAINS_MONO)?;

    // Load code.

    // spellcheck: ignore
    // let code: AttributedCode =
    //     serde_json::from_str(&fs::read_to_string("/tmp/code.json").unwrap()).unwrap();
    let code: AttributedText = postcard::from_bytes(include_bytes!("code.postcard")).unwrap();

    // Shape and layout text.

    let font_size = 32.;
    let line_height = 40.;
    // let font_size = 16.;
    // let line_height = 20.;

    // The handle is `!Send` and is dropped at the end of this block, before any await, so the
    // example's future stays `Send` for `Application::new`.
    let (glyph_runs, height) = {
        let mut shaper = shaper();
        attributed_text::shape_text(
            &mut shaper,
            &code.text,
            &code.attributes,
            font_size,
            line_height,
            None,
        )
    };

    // View

    let initial_size = LogicalSize::new(800., 800.).to_physical(ctx.primary_monitor_scale_factor());
    let view_size = SizePx::new(initial_size.width, initial_size.height);
    let mut view = ctx.view(view_size).build()?;
    let mut center = ViewCenter::new(&view, view_size);

    let content_size = SizePx::new(1280, height as u32);
    let mut gestures = GestureTransform::default();
    let mut transform = gestures.get_transform(content_size).submit();
    let location = transform
        .to_location()
        .relative_to(center.location())
        .submit();

    // Hold the visual in this context, otherwise it will disappear.
    let _visual = glyph_runs
        .into_iter()
        .map(|m| m.into())
        .collect::<Vec<_>>()
        .at(&location)
        .submit();

    loop {
        transform.update_if_changed(gestures.get_transform(content_size));
        ctx.submit(begin_frame().submission())?;

        for event in ctx.wait_for_events::<Infallible>().await? {
            info!("Event: {event:?}");

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
