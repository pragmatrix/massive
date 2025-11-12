use std::{
    collections::{HashMap, VecDeque},
    mem,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use cosmic_text::FontSystem;
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
use winit::dpi::LogicalSize;

use massive_geometry::{SizeI, Vector3};
use massive_scene::Visual;
use massive_shell::{ApplicationContext, FontManager, Scene, shell};
use shared::{
    application::{Application, UpdateResponse},
    positioning,
};

const CANVAS_ID: &str = "massive-emojis";

#[tokio::main]
async fn main() -> Result<()> {
    shell::run(emojis)
}

async fn emojis(mut ctx: ApplicationContext) -> Result<()> {
    let markdown = include_str!("emojis.md");
    // The concepts of a current dir does not exist in wasm I guess.
    // let current_dir = env::current_dir().expect("Failed to get current directory");
    // let file_path: PathBuf = [current_dir.to_str().unwrap(), "replicator.org.md"]
    //     .iter()
    //     .collect();

    let theme = Theme::light_default();
    let html = markdown_to_html(markdown, theme.code_highlighter.clone());

    let element_queue = Arc::new(Mutex::new(VecDeque::new()));

    let fonts = FontManager::system();
    // Need an equivalent FontSystem for inlyne.
    let font_system = Arc::new(Mutex::new(FontSystem::new()));

    let initial_size = LogicalSize::new(1280., 800.);

    let window = ctx.new_window(initial_size, Some(CANVAS_ID)).await?;

    let mut renderer = window.renderer().with_text(fonts).build().await?;

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
        font_system: font_system.clone(),
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

    // Application

    let page_size = SizeI::new(page_width as _, page_height);
    let mut application = Application::default();
    let scene = Scene::new();
    let matrix = scene.stage(application.matrix(page_size));
    let location = scene.stage(matrix.clone().into());

    // Hold the staged visual, otherwise it will disappear.
    let _visual = scene.stage(Visual::new(
        location.clone(),
        glyph_runs
            .into_iter()
            .map(|run| run.into())
            .collect::<Vec<_>>(),
    ));

    loop {
        let event = ctx.wait_for_shell_event().await?;

        info!("Event: {event:?}");

        if let Some(window_event) = event.window_event_for_id(window.id()) {
            match application.update(window_event) {
                UpdateResponse::Exit => return Ok(()),
                UpdateResponse::Continue => {}
            }
        }

        matrix.update_if_changed(application.matrix(page_size));

        scene.render_to(&mut renderer, Some(event))?;
    }
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

#[cfg(test)]
mod tests {
    // Somehow swash can't render NotoColorEmoji
    // Placement {
    //     left: -1,
    //     top: 0,
    //     width: 2,
    //     height: 0,
    // }

    #[test]
    fn test_color_emoji_font() {
        use swash::FontRef;
        use swash::scale::ScaleContext;
        use swash::scale::*;
        use swash::zeno;
        use zeno::Vector;

        let font_data = include_bytes!("fonts/NotoColorEmoji-Regular.ttf");
        let font = FontRef::from_index(font_data, 0).unwrap();

        let mut context = ScaleContext::new();
        let mut scaler = context.builder(font).hint(true).size(100.0).build();
        let glyph_id = font.charmap().map('😀');
        let image = Render::new(&[
            // Source::ColorBitmap(StrikeWith::BestFit),
            // Source::ColorOutline(0),
            Source::Outline,
        ])
        .format(zeno::Format::Alpha)
        .offset(Vector::new(0.0, 0.0))
        .render(&mut scaler, glyph_id)
        .unwrap();

        dbg!(image.placement);

        // let img =
        //     ::image::RgbaImage::from_raw(image.placement.width, image.placement.height, image.data)
        //         .unwrap();
        // _ = img.save_with_format("/tmp/stuff.png", ::image::ImageFormat::Png);
    }
}
