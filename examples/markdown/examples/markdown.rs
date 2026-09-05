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

use massive_applications::ApplicationEvent;
use massive_geometry::SizePx;
use massive_scene::prelude::*;
use massive_shapes::GlyphRun;
use massive_shell::shell;
use massive_shell::{ApplicationContext, FontManager};

use shared::application::{Application, UpdateResponse};
use shared::fonts;

use markdown::cosmic_buffer_to_glyph_runs;

#[tokio::main]
async fn main() -> Result<()> {
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

    shell::run(application)
}

async fn application(mut ctx: ApplicationContext) -> Result<()> {
    let fonts = FontManager::bare().with_font(fonts::MONTSERRAT_REGULAR);

    // Need an equivalent font_system for inlyne.
    let font_system = {
        // Don't load system fonts for now, this way we get the same result on wasm and local runs.
        let mut font_db = fontdb::Database::new();
        let montserrat = fonts::MONTSERRAT_REGULAR;
        let source = fontdb::Source::Binary(Arc::new(montserrat));
        font_db.load_font_source(source);
        FontSystem::new_with_locale_and_db("en-US".into(), font_db)
    };

    let scale_factor = ctx.primary_monitor_scale_factor();
    let physical_size = LogicalSize::new(960, 800).to_physical(scale_factor);
    let window = ctx
        .new_window((physical_size.width, physical_size.height))
        .await?;
    let view_id = window.view_id();

    let font_system = Arc::new(Mutex::new(font_system));

    let mut renderer = window.renderer().with_text(fonts.clone()).build().await?;

    let markdown = include_str!("replicator.org.md");

    let (glyph_runs, content_size) =
        markdown_to_glyph_runs(&fonts, scale_factor, physical_size, font_system.clone(), markdown)?;

    let mut application = Application::default();
    let scene = ctx.new_scene();
    let page_transform = application.get_transform(content_size);

    let transform = page_transform.enter(&scene);
    let location = transform.to_location().enter(&scene);

    // Hold the staged visual, otherwise it will disappear.
    let _visual = glyph_runs
        .clone()
        .into_iter()
        .map(|run| run.into())
        .collect::<Vec<_>>()
        .at(&location)
        .with_decal_order(0)
        .enter(&scene);

    loop {
        for event in ctx.wait_for_events::<Infallible>().await? {
            match event {
                ApplicationEvent::View(event_view_id, view_event) if event_view_id == view_id => {
                    info!("View Event: {view_event:?}");

                    match application.update(&view_event) {
                        UpdateResponse::Exit => {
                            info!("Exiting Markdown application");
                            return Ok(());
                        }
                        UpdateResponse::Continue => {}
                    }

                    transform.update_if_changed(application.get_transform(content_size));
                    renderer.resize_redraw(&view_event)?;
                }
                ApplicationEvent::View(..)
                | ApplicationEvent::ApplyAnimations(_)
                | ApplicationEvent::Shutdown(_) => {}
                ApplicationEvent::Custom(event) => match event {},
            }
        }

        ctx.frame(&scene).render_to(&mut renderer)?;
    }
}

fn markdown_to_glyph_runs(
    fonts: &FontManager,
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
                cosmic_buffer_to_glyph_runs(fonts, text_area.buffer, text_area.left, text_area.top)
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
