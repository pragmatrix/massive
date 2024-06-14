use std::{
    collections::{HashMap, VecDeque},
    mem,
    sync::{Arc, Mutex},
};

use anyhow::{bail, Context, Result};
use cosmic_text::{fontdb, FontSystem};
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
use massive_scene::PositionedShape;
use massive_shapes::GlyphRun;

use massive_geometry::{Camera, SizeI, Vector3};
use massive_shell::{ApplicationContext, Shell2};

use shared::{
    application,
    application2::{Application2, UpdateResponse},
    fonts, positioning,
};

// Explicitly provide the id of the canvas to use (don't like this hidden magic with data-raw-handle)
const CANVAS_ID: &str = "massive-markdown";

fn main() -> Result<()> {
    shared::main(markdown)
}

async fn markdown() -> Result<()> {
    let event_loop = Shell2::event_loop()?;
    let window = application::create_window(&event_loop, Some(CANVAS_ID))?;
    info!("Initial Window inner size: {:?}", window.inner_size());

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

    #[cfg(not(target_arch = "wasm32"))]
    let initial_size = window.inner_size();
    // On wasm, the initial size is always, 0,0, so we set one (this is also used for the page
    // layout) and leave it to subsequent resize events to configure the proper size.
    #[cfg(target_arch = "wasm32")]
    let initial_size = winit::dpi::PhysicalSize::new(1280, 800);

    let font_system = Arc::new(Mutex::new(font_system));
    let mut shell = Shell2::new(&window, initial_size, font_system.clone()).await?;

    // Camera

    let camera = {
        let fovy: f64 = 45.0;
        let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();
        Camera::new((0.0, 0.0, camera_distance), (0.0, 0.0, 0.0))
    };

    // Run

    shell.run(event_loop, &window, camera, application).await
}

async fn application(mut ctx: ApplicationContext) -> Result<()> {
    let markdown = include_str!("replicator.org.md");

    let (glyph_runs, page_size) = markdown_to_glyph_runs(&ctx, markdown)?;

    let mut director = ctx.director();

    let mut application = Application2::new(page_size);
    let matrix = director.cast(application.matrix());

    // Hold the positioned shapes in this context, otherwise they will disappear.
    let _positioned_shapes: Vec<_> = glyph_runs
        .into_iter()
        .map(|run| director.cast(PositionedShape::new(matrix.clone(), run)))
        .collect();

    director.action()?;

    loop {
        if let Some(event) = ctx.window_events.recv().await {
            info!("Application Event: {event:?}");
            match application.update(event) {
                UpdateResponse::Continue => {}
                UpdateResponse::Exit => return Ok(()),
            }

            matrix.update(application.matrix());

            director.action()?;
        } else {
            // TODO: clarify when this happens. When the window is closed?
            bail!("No more window events .. ???")
        }
    }
}

fn markdown_to_glyph_runs(
    ctx: &ApplicationContext,
    markdown: &str,
) -> Result<(Vec<GlyphRun>, SizeI)> {
    let theme = Theme::light_default();
    let html = markdown_to_html(markdown, theme.code_highlighter.clone());

    let element_queue = Arc::new(Mutex::new(VecDeque::new()));
    let image_cache = Arc::new(Mutex::new(HashMap::new()));
    let color_scheme = Some(ResolvedTheme::Light);

    let window_scale_factor = ctx.window_scale_factor;

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

    let initial_size = ctx.initial_window_size;
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
        font_system: ctx.font_system.clone(),
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
