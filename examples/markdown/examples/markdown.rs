use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::mem;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use log::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};
use winit::dpi::{LogicalSize, PhysicalSize};

use cosmic_text::FontSystem;
use cosmic_text::fontdb;
use inlyne::Element;
use inlyne::color::Theme;
use inlyne::interpreter::HtmlInterpreter;
use inlyne::opts::ResolvedTheme;
use inlyne::positioner::DEFAULT_MARGIN;
use inlyne::positioner::{Positioned, Positioner};
use inlyne::text::{CachedTextArea, TextCache, TextSystem};
use inlyne::utils::Rect;
use inlyne::utils::markdown_to_html;

use massive_applications::prelude::*;
use massive_applications::{Application, ApplicationEvent, InstanceContext, ViewEvent};
use massive_geometry::SizePx;
use massive_scene::prelude::*;
use massive_shapes::GlyphRun;
use massive_shapes::{FontPolicy, ShapingEngineKind};
use massive_shell::shell;

use shared::fonts;
use shared::gesture_transform::{GestureTransform, UpdateResponse};
use shared::view_center::ViewCenter;

use markdown::FontBridge;

fn main() -> Result<()> {
    let env_filter = EnvFilter::from_default_env();
    let console_formatter = tracing_subscriber::fmt::Layer::default();
    // let (flame_layer, _flame_guard) = FlameLayer::with_file("./tracing.folded").unwrap();

    let now: DateTime<Local> = Local::now();
    #[allow(unused)]
    let time_code = now.format("%Y%m%d%H%M").to_string();

    // let (chrome_layer, _chrome_guard) = tracing_chrome::ChromeLayerBuilder::new()
    //     .file(format!("./massive-trace-{time_code}.json"))
    //     .build();

    Registry::default()
        // Filter seems to be applied globally, which is what we want.
        .with(env_filter)
        // Console formatter currently captures only log::xxx! macros for some reason.
        .with(console_formatter)
        // .with(flame_layer)
        // .with(chrome_layer)
        .init();

    shell::run(
        Application::new("Markdown", markdown),
        FontPolicy::bare(ShapingEngineKind::CosmicText),
    )
}

async fn markdown(mut ctx: InstanceContext) -> Result<()> {
    // Register the bundled font into both databases and build the fontdb::ID -> FaceId map; the
    // task's manager is the one identity world the renderer reads (ADR 0005).
    let bridge = FontBridge::new(
        fontdb::Database::new(),
        Arc::from(fonts::MONTSERRAT_REGULAR),
    )?;

    // Need an equivalent font_system for inlyne.
    let font_system = {
        // Don't load system fonts for now, this way we get the same result on wasm and local runs.
        FontSystem::new_with_locale_and_db("en-US".into(), bridge.font_db().clone())
    };

    let scale_factor = ctx.primary_monitor_scale_factor();
    let physical_size = LogicalSize::new(960, 800).to_physical(scale_factor);
    let view_size = SizePx::new(physical_size.width, physical_size.height);
    let view = ctx.view(view_size).build()?;
    let center = ViewCenter::new(&view, view_size);

    let font_system = Arc::new(Mutex::new(font_system));

    let markdown = include_str!("replicator.org.md");

    let (glyph_runs, content_size) = markdown_to_glyph_runs(
        &bridge,
        scale_factor,
        physical_size,
        font_system.clone(),
        markdown,
    )?;

    let mut gestures = GestureTransform::default();
    let page_transform = gestures.get_transform(content_size);

    let transform = page_transform.submit();
    let location = transform
        .to_location()
        .relative_to(center.location())
        .submit();

    // Hold the submitted visual, otherwise it will disappear.
    let _visual = glyph_runs
        .clone()
        .into_iter()
        .map(|run| run.into())
        .collect::<Vec<_>>()
        .at(&location)
        .with_decal_order(0)
        .submit();

    loop {
        transform.update_if_changed(gestures.get_transform(content_size));
        ctx.submit(begin_frame().submission())?;

        for event in ctx.wait_for_events::<Infallible>().await? {
            match event {
                ApplicationEvent::View(_, view_event) => {
                    info!("View Event: {view_event:?}");

                    if let ViewEvent::Resized(size) = view_event {
                        center.resize(&view, size);
                    }
                    match gestures.update(&view_event) {
                        UpdateResponse::Exit => {
                            info!("Exiting Markdown application");
                            return Ok(());
                        }
                        UpdateResponse::Continue => {}
                    }
                }
                ApplicationEvent::Shutdown(_) => return Ok(()),
                ApplicationEvent::ApplyAnimations(_)
                | ApplicationEvent::FullscreenRequested
                | ApplicationEvent::ToolCall(_)
                | ApplicationEvent::CancelToolCall(_) => {}
                ApplicationEvent::Custom(event) => match event {},
            }
        }
    }
}

fn markdown_to_glyph_runs(
    bridge: &FontBridge,
    window_scale_factor: f64,
    content_size: PhysicalSize<u32>,
    font_system: Arc<Mutex<FontSystem>>,
    markdown: &str,
) -> Result<(Vec<GlyphRun>, SizePx)> {
    let theme = Theme::light_default();
    let html = markdown_to_html(markdown, theme.code_highlighter.clone());

    let element_queue = Arc::new(Mutex::new(VecDeque::new()));
    let image_cache = Arc::new(Mutex::new(HashMap::new()));
    let color_scheme = Some(ResolvedTheme::Light);

    let interpreter = HtmlInterpreter::new_with_interactor_granularity(
        element_queue.clone(),
        theme,
        window_scale_factor as _,
        // file_path,
        image_cache,
        color_scheme,
    );

    interpreter.interpret_html(&html);

    let elements = {
        let mut elements_queue = element_queue.lock().unwrap();
        mem::take(&mut *elements_queue)
    };

    let initial_size = content_size;
    let width = initial_size.width;
    let page_width = width;

    let mut positioner = Positioner::new(
        (width as _, initial_size.height as _),
        window_scale_factor as _,
        page_width as _,
    );

    let text_cache = Arc::new(Mutex::new(TextCache::new()));

    let mut elements: Vec<Positioned<Element>> =
        elements.into_iter().map(Positioned::new).collect();

    let mut text_system = TextSystem {
        font_system,
        text_cache: text_cache.clone(),
    };

    let zoom = 1.0;
    positioner.reposition(&mut text_system, &mut elements, zoom)?;

    let screen_size = (width as f32, f32::INFINITY);
    let scroll_y = 0.;

    let cached_text_areas = get_text_areas(
        &mut text_system,
        screen_size,
        zoom,
        page_width as _,
        scroll_y,
        &elements,
    )?;

    let mut glyph_runs = Vec::new();
    let mut page_height = 0;

    {
        let text_cache = text_cache.lock().unwrap();

        let text_areas = {
            cached_text_areas
                .iter()
                .map(|cta| cta.text_area(&text_cache))
        };

        // Note: text_area.bounds are not set (for some reason?).
        for text_area in text_areas {
            let line_height = text_area.buffer.metrics().line_height;
            for glyph_run in
                bridge.cosmic_buffer_to_glyph_runs(text_area.buffer, text_area.left, text_area.top)
            {
                let top = glyph_run.translation.y as f32;
                glyph_runs.push(glyph_run);

                page_height = (top + line_height).ceil() as _;
            }
        }
    }

    Ok((glyph_runs, (page_width, page_height).into()))
}

// #[derive(Debug)]
// struct Interactor {}

// impl WindowInteractor for Interactor {
//     fn finished_single_doc(&self) {}

//     fn request_redraw(&self) {}

//     fn image_callback(&self) -> Box<dyn inlyne::interpreter::ImageCallback + Send> {
//         println!("Interactor: Acquiring image callback");
//         Box::new(ImageCallbackImpl {})
//     }
// }

// #[derive(Debug)]
// struct ImageCallbackImpl {}

// impl ImageCallback for ImageCallbackImpl {
//     fn loaded_image(&self, src: String, _image_data: Arc<Mutex<Option<inlyne::image::ImageData>>>) {
//         println!("Interactor.ImageCallback: Loaded Image {}", src)
//     }
// }

// A stripped down port of the `inlyne::renderer::render_elements` function.
fn get_text_areas(
    text_system: &mut TextSystem,
    screen_size: (f32, f32),
    zoom: f32,
    page_width: f32,
    scroll_y: f32,
    elements: &[Positioned<Element>],
) -> Result<Vec<CachedTextArea>> {
    let mut text_areas: Vec<CachedTextArea> = Vec::new();

    let centering = (screen_size.0 - page_width).max(0.) / 2.;

    for element in elements {
        let Rect { pos, size: _ } = element.bounds.as_ref().context("Element not positioned")?;

        match &element.inner {
            Element::TextBox(text_box) => {
                let bounds = (
                    (screen_size.0 - pos.0 - DEFAULT_MARGIN - centering).max(0.),
                    f32::INFINITY,
                );

                let areas = text_box.text_areas(text_system, *pos, bounds, zoom, scroll_y);
                text_areas.push(areas);
            }
            Element::Spacer(_) => {}
            Element::Image(_) => todo!(),
            Element::Table(_) => todo!(),
            Element::Row(_) => todo!(),
            Element::Section(_) => todo!(),
        }
    }

    Ok(text_areas)
}
