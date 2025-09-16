use std::{
    collections::{HashMap, VecDeque},
    mem,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use cosmic_text::{FontSystem, fontdb};
use inlyne::{
    Element,
    color::Theme,
    interpreter::HtmlInterpreter,
    opts::ResolvedTheme,
    positioner::{DEFAULT_MARGIN, Positioned, Positioner},
    text::{CachedTextArea, TextCache, TextSystem},
    utils::{Rect, markdown_to_html},
};
use log::info;
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt, util::SubscriberInitExt};
use winit::dpi::PhysicalSize;

use massive_geometry::{Camera, SizeI, Vector3};
use massive_scene::Visual;
use massive_shapes::GlyphRun;
use massive_shell::{ApplicationContext, Scene, shell};
use shared::{
    application::{Application, UpdateResponse},
    fonts, positioning,
};

// Explicitly provide the id of the canvas to use (don't like this hidden magic with data-raw-handle)
const CANVAS_ID: &str = "massive-markdown";

#[tokio::main]
async fn main() -> Result<()> {
    let env_filter = EnvFilter::from_default_env();
    let console_formatter = tracing_subscriber::fmt::Layer::default();
    // let (flame_layer, _flame_guard) = FlameLayer::with_file("./tracing.folded").unwrap();

    let now: DateTime<Local> = Local::now();
    #[allow(unused)]
    let time_code = now.format("%Y%m%d%H%M").to_string();

    let (chrome_layer, _chrome_guard) = tracing_chrome::ChromeLayerBuilder::new()
        .file(format!("./massive-trace-{time_code}.json"))
        .build();

    Registry::default()
        // Filter seems to be applied globally, which is what we want.
        .with(env_filter)
        // Console formatter currently captures only log::xxx! macros for some reason.
        .with(console_formatter)
        // .with(flame_layer)
        .with(chrome_layer)
        .init();

    shell::run(application)
}

async fn application(mut ctx: ApplicationContext) -> Result<()> {
    let font_system = {
        // In wasm the system locale can't be acquired. `sys_locale::get_locale()`
        const DEFAULT_LOCALE: &str = "en-US";

        // Don't load system fonts for now, this way we get the same result on wasm and local runs.
        let mut font_db = fontdb::Database::new();
        let montserrat = fonts::MONTSERRAT_REGULAR;
        let source = fontdb::Source::Binary(Arc::new(montserrat));
        font_db.load_font_source(source);
        FontSystem::new_with_locale_and_db(DEFAULT_LOCALE.into(), font_db)
    };

    // Camera

    let camera = {
        let fovy: f64 = 45.0;
        let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();
        Camera::new((0.0, 0.0, camera_distance), (0.0, 0.0, 0.0))
    };

    let initial_size = winit::dpi::LogicalSize::new(960, 800);
    let window = ctx.new_window(initial_size, Some(CANVAS_ID)).await?;
    let scale_factor = window.scale_factor();
    let physical_size = initial_size.to_physical(scale_factor);

    let font_system = Arc::new(Mutex::new(font_system));

    let mut renderer = window
        .new_renderer(
            font_system.clone(),
            camera,
            initial_size.to_physical(scale_factor),
        )
        .await?;

    let markdown = include_str!("replicator.org.md");

    let (glyph_runs, page_size) =
        markdown_to_glyph_runs(scale_factor, physical_size, font_system.clone(), markdown)?;

    let mut application = Application::default();
    let scene = Scene::new();
    let page_matrix = application.matrix(page_size);

    let matrix = scene.stage(page_matrix);
    let location = scene.stage(matrix.clone().into());

    // Hold the staged visual, otherwise it will disappear.
    let _visual = scene.stage(Visual::new(
        location,
        glyph_runs
            .clone()
            .into_iter()
            .map(|run| run.into())
            .collect::<Vec<_>>(),
    ));

    loop {
        let event = ctx.wait_for_shell_event(&mut renderer).await?;

        let window_id = renderer.window_id();

        let _cycle = scene.begin_update_cycle(&mut renderer, Some(&event))?;

        if let Some(window_event) = event.window_event_for_id(window_id) {
            info!("Window Event: {window_event:?}");

            match application.update(window_event) {
                UpdateResponse::Exit => {
                    info!("Exiting Markdown application");
                    return Ok(());
                }
                UpdateResponse::Continue => {}
            }

            matrix.update_if_changed(application.matrix(page_size));
        }
    }
}

fn markdown_to_glyph_runs(
    window_scale_factor: f64,
    page_size: PhysicalSize<u32>,
    font_system: Arc<Mutex<FontSystem>>,
    markdown: &str,
) -> Result<(Vec<GlyphRun>, SizeI)> {
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

    let initial_size = page_size;
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
            for run in text_area.buffer.layout_runs() {
                let top = text_area.top + run.line_top;
                let translation = Vector3::new(text_area.left as _, top as _, 0.0);
                let glyph_run = positioning::to_glyph_run(translation, &run, line_height);

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
