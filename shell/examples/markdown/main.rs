use std::{
    collections::{HashMap, VecDeque},
    mem,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use cosmic_text::{fontdb, CacheKey, CacheKeyFlags, FontSystem, LayoutGlyph};
use inlyne::{
    color::Theme,
    interpreter::HtmlInterpreter,
    opts::ResolvedTheme,
    positioner::{Positioned, Positioner, DEFAULT_MARGIN},
    text::{CachedTextArea, TextCache, TextSystem},
    utils::{markdown_to_html, Rect},
    Element,
};
use log::info;
use winit::{
    event_loop::EventLoop,
    window::{Window, WindowBuilder},
};

use application::Application;

#[cfg(target_arch = "wasm32")]
use winit::platform::web::WindowBuilderExtWebSys;

use massive_geometry::{Camera, Point, SizeI};
use massive_shapes::{GlyphRun, GlyphRunMetrics, PositionedGlyph};
use massive_shell::Shell;

#[path = "../shared/application.rs"]
mod application;

#[path = "../shared/positioning.rs"]
mod positioning;

// Explicitly provide the id of the canvas to use (don't like this hidden magic with data-raw-handle)
const CANVAS_ID: &str = "markdown";

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
    let markdown = include_str!("replicator.org.md");
    // The concepts of a current dir does not exist in wasm I guess.
    // let current_dir = env::current_dir().expect("Failed to get current directory");
    // let file_path: PathBuf = [current_dir.to_str().unwrap(), "replicator.org.md"]
    //     .iter()
    //     .collect();

    let theme = Theme::light_default();
    let html = markdown_to_html(markdown, theme.code_highlighter.clone());

    let element_queue = Arc::new(Mutex::new(VecDeque::new()));

    let event_loop = EventLoop::new()?;
    let window = application::create_window(&event_loop, Some(CANVAS_ID))?;
    info!("Initial Window inner size: {:?}", window.inner_size());

    let font_system = {
        // In wasm the system locale can't be acquired. `sys_locale::get_locale()`
        const DEFAULT_LOCALE: &str = "en-US";

        // Don't load system fonts for now, this way we get the same result on wasm and local runs.
        let mut font_db = fontdb::Database::new();
        let montserrat = include_bytes!("Montserrat-Regular.ttf");
        let source = fontdb::Source::Binary(Arc::new(montserrat));
        font_db.load_font_source(source);
        FontSystem::new_with_locale_and_db(DEFAULT_LOCALE.into(), font_db)
    };

    #[cfg(not(target_arch = "wasm32"))]
    let initial_size = window.inner_size();
    // On wasm, the initial size is always, 0,0, so we set one (this is also used for the page
    // layout) and leave it to subsequent resize events to configure the proper size.
    #[cfg(target_arch = "wasm32")]
    let initial_size = winit::dpi::PhysicalSize::new(1280, 800);

    let font_system = Arc::new(Mutex::new(font_system));
    let mut shell = Shell::new(&window, initial_size, font_system.clone()).await?;

    // TODO: Pass surface format.
    let _surface_format = shell.surface_format();
    let hidpi_scale = window.scale_factor();
    let image_cache = Arc::new(Mutex::new(HashMap::new()));

    let color_scheme = Some(ResolvedTheme::Light);

    let interpreter = HtmlInterpreter::new_with_interactor_granularity(
        element_queue.clone(),
        theme,
        hidpi_scale as _,
        // file_path,
        image_cache,
        color_scheme,
    );

    interpreter.interpret_html(&html);

    let elements = {
        let mut elements_queue = element_queue.lock().unwrap();
        mem::take(&mut *elements_queue)
    };

    let width = initial_size.width;
    let page_width = width;

    let mut positioner = Positioner::new(
        (width as _, initial_size.height as _),
        hidpi_scale as _,
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
        for text_area in text_areas.take(10) {
            let line_height = text_area.buffer.metrics().line_height;
            for run in text_area.buffer.layout_runs() {
                let glyph_run = positioning::to_glyph_run(&run, line_height);

                let top = text_area.top + run.line_top;
                let offset = Point::new(text_area.left as _, top as _);

                glyph_runs.push((offset, glyph_run));

                page_height = (top + line_height).ceil() as _;
            }
        }
    }

    // Camera

    let camera = {
        let fovy: f64 = 45.0;
        let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();
        Camera::new((0.0, 0.0, camera_distance), (0.0, 0.0, 0.0))
    };

    // Application

    let application =
        Application::new(camera, glyph_runs, SizeI::new(page_width as _, page_height));

    // Run

    shell.run(event_loop, &window, application).await
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
