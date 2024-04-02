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
    event::{
        DeviceId, KeyEvent, Modifiers, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
    },
    event_loop::EventLoop,
    keyboard::{Key, NamedKey},
    window::{Window, WindowBuilder},
};

#[cfg(target_arch = "wasm32")]
use winit::platform::web::WindowBuilderExtWebSys;

use massive_geometry::{Camera, Matrix4, Point, PointI, SizeI, Vector3};
use massive_shapes::{GlyphRun, GlyphRunMetrics, PositionedGlyph, Shape};
use massive_shell::{self as shell, Shell};

// Explicitly provide the id of the canvas to use (don't like this hidden magic with data-raw-handle)
#[cfg(target_arch = "wasm32")]
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
    let window = create_window(&event_loop)?;
    info!("Initial Window inner size: {:?}", window.inner_size());

    let font_system = {
        // In wasm the system locale can't be acquired. `sys_locale::get_locale()`
        const DEFAULT_LOCALE: &str = "en-US";

        // Don't load system fonts for now, this way we get the same result on wasm and local runs.
        let mut font_db = fontdb::Database::new();
        let montserrat = include_bytes!("Montserrat-Regular.ttf");
        let source = fontdb::Source::Binary(Arc::new(montserrat));
        font_db.load_font_source(source);
        Arc::new(Mutex::new(FontSystem::new_with_locale_and_db(
            DEFAULT_LOCALE.into(),
            font_db,
        )))
    };

    #[cfg(not(target_arch = "wasm32"))]
    fn create_window(event_loop: &EventLoop<()>) -> Result<Window> {
        Ok(WindowBuilder::new().build(event_loop)?)
    }

    // Explicitly query for the canvas, and initialize the window with it.
    //
    // If we use the implicit of `data-raw-handle="1"`, no resize event will be sent.
    #[cfg(target_arch = "wasm32")]
    fn create_window(event_loop: &EventLoop<()>) -> Result<Window> {
        use wasm_bindgen::JsCast;

        let canvas = web_sys::window()
            .expect("No Window")
            .document()
            .expect("No document")
            .query_selector(&format!("#{CANVAS_ID}"))
            // what a shit-show here, why is the error not compatible with anyhow.
            .map_err(|err| anyhow::anyhow!(err.as_string().unwrap()))?
            .expect("No Canvas with a matching id found");

        let canvas: web_sys::HtmlCanvasElement = canvas
            .dyn_into()
            .map_err(|_| anyhow::anyhow!("Failed to cast to HtmlCanvasElement"))?;

        Ok(WindowBuilder::new()
            .with_canvas(Some(canvas))
            .build(event_loop)?)
    }

    #[cfg(not(target_arch = "wasm32"))]
    let initial_size = window.inner_size();
    // On wasm, the initial size is always, 0,0, so we set one (this is also used for the page
    // layout) and leave it to subsequent resize events to configure the proper size.
    #[cfg(target_arch = "wasm32")]
    let initial_size = winit::dpi::PhysicalSize::new(1280, 800);

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
                let max_ascent = run.line_y - run.line_top;

                let glyph_run_metrics = GlyphRunMetrics {
                    max_ascent: max_ascent.ceil() as _,
                    max_descent: (line_height - max_ascent).ceil() as _,
                    width: run.line_w.ceil() as u32,
                };

                let positioned = position_glyphs(run.glyphs);

                let top = text_area.top + run.line_top;
                let offset = Point::new(text_area.left as _, top as _);

                glyph_runs.push((offset, GlyphRun::new(glyph_run_metrics, positioned)));

                page_height = (top + line_height).ceil() as _;

                // println!("run: {:?}", run);
                // for glyph in run.glyphs.iter() {
                //     println!("lt: {}, {}", text_area.left, text_area.top);
                //     let physical_glyph =
                //         glyph.physical((text_area.left, text_area.top), text_area.scale);
                //     println!(
                //         "ck: {:?} {} {}",
                //         physical_glyph.cache_key, physical_glyph.x, physical_glyph.y
                //     );
                // }
            }
        }
    }

    // println!("text areas: {}", text_areas.len());

    let fovy: f64 = 45.0;
    let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();
    let camera = Camera::new((0.0, 0.0, camera_distance), (0.0, 0.0, 0.0));

    let application = Application {
        camera,
        glyph_runs,
        page_size: SizeI::new(page_width as _, page_height),
        left_mouse_button_pressed: None,
        positions: HashMap::new(),
        translation: PointI::default(),
        translation_z: 0,
        rotation: PointI::default(),
        modifiers: Modifiers::default(),
    };

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

const RENDER_SUBPIXEL: bool = false;

fn position_glyphs(glyphs: &[LayoutGlyph]) -> Vec<PositionedGlyph> {
    glyphs
        .iter()
        .map(|glyph| {
            let fractional_pos = if RENDER_SUBPIXEL {
                (glyph.x, glyph.y)
            } else {
                (glyph.x.round(), glyph.y.round())
            };

            let (ck, x, y) = CacheKey::new(
                glyph.font_id,
                glyph.glyph_id,
                glyph.font_size,
                fractional_pos,
                CacheKeyFlags::empty(),
            );
            // Note: hitbox with is fractional, but does not change with / without subpixel
            // rendering.
            PositionedGlyph::new(ck, (x, y), glyph.w)
        })
        .collect()
}

struct Application {
    camera: Camera,
    glyph_runs: Vec<(Point, GlyphRun)>,
    page_size: SizeI,

    /// If pressed, the origin.
    left_mouse_button_pressed: Option<MouseButtonPressed>,
    /// Tracked positions of all devices.
    positions: HashMap<DeviceId, PointI>,
    modifiers: Modifiers,

    /// Current x / y Translation.
    translation: PointI,
    translation_z: i32,
    /// Rotation in discrete degrees.
    rotation: PointI,
}

struct MouseButtonPressed {
    device_id: DeviceId,
    origin: PointI,
    translation_origin: PointI,
    rotation_origin: PointI,
}

const MOUSE_WHEEL_SCROLL_TO_Z_PIXELS: i32 = 16;

impl shell::Application for Application {
    fn update(&mut self, window_event: WindowEvent) {
        match window_event {
            WindowEvent::CursorMoved {
                device_id,
                position,
            } => {
                // track
                // These positions aren't discrete on macOS, but why?
                let current = PointI::new(position.x.round() as _, position.y.round() as _);
                self.positions.insert(device_id, current);

                // ongoing movement?
                if let Some(pressed_state) = &self.left_mouse_button_pressed {
                    let delta = current - pressed_state.origin;

                    if self.modifiers.state().control_key() {
                        self.rotation = pressed_state.rotation_origin + delta;
                    } else {
                        self.translation = pressed_state.translation_origin + delta;
                    }
                }
            }
            WindowEvent::MouseWheel {
                delta: MouseScrollDelta::LineDelta(_, y_delta),
                phase: TouchPhase::Moved,
                ..
            } => self.translation_z += y_delta.round() as i32 * MOUSE_WHEEL_SCROLL_TO_Z_PIXELS,
            WindowEvent::MouseInput {
                device_id,
                state,
                button: MouseButton::Left,
            } if self.positions.contains_key(&device_id) => {
                if state.is_pressed() {
                    self.left_mouse_button_pressed = Some(MouseButtonPressed {
                        device_id,
                        origin: self.positions[&device_id],
                        translation_origin: self.translation,
                        rotation_origin: self.rotation,
                    });
                } else {
                    self.left_mouse_button_pressed = None
                }
            }
            WindowEvent::MouseInput {
                button: MouseButton::Middle,
                ..
            } => {
                self.rotation = PointI::default();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                if self.modifiers != modifiers {
                    // If there is an ongoing move and modifiers change, reset origins.
                    if let Some(ref mut mouse_pressed) = self.left_mouse_button_pressed {
                        mouse_pressed.origin = self.positions[&mouse_pressed.device_id];
                        mouse_pressed.translation_origin = self.translation;
                        mouse_pressed.rotation_origin = self.rotation;
                    }

                    self.modifiers = modifiers
                }
            }
            WindowEvent::KeyboardInput {
                event: KeyEvent {
                    logical_key, state, ..
                },
                ..
            } => {
                if state == winit::event::ElementState::Pressed {
                    match logical_key {
                        Key::Named(NamedKey::ArrowLeft) => {
                            self.camera.eye += Vector3::new(0.1, 0.0, 0.0)
                        }
                        Key::Named(NamedKey::ArrowRight) => {
                            self.camera.eye -= Vector3::new(0.1, 0.0, 0.0)
                        }
                        Key::Named(NamedKey::ArrowUp) => {
                            self.camera.eye += Vector3::new(0.0, 0.0, 0.1)
                        }
                        Key::Named(NamedKey::ArrowDown) => {
                            self.camera.eye -= Vector3::new(0.0, 0.0, 0.1)
                        }
                        _ => {}
                    }
                } else {
                    {}
                }

                println!("eye: {:?}", self.camera.eye)
            }
            _ => (),
        }
    }

    fn render(&self, shell: &mut Shell) -> (Camera, Vec<Shape>) {
        let mut shapes = Vec::new();

        let page_x_center: f64 = -((self.page_size.width / 2) as f64);
        let page_y_center: f64 = -((self.page_size.height / 2) as f64);
        let center_transformation =
            Matrix4::from_translation((page_x_center, page_y_center, 0.0).into());
        let current_translation = Matrix4::from_translation(
            (
                self.translation.x as _,
                self.translation.y as _,
                self.translation_z as _,
            )
                .into(),
        );
        let angle_x = cgmath::Rad((self.rotation.x as f64 / 10.).to_radians());
        let angle_y = cgmath::Rad((-self.rotation.y as f64 / 10.).to_radians());

        let x_rotation = Matrix4::from_angle_y(angle_x);
        let y_rotation = Matrix4::from_angle_x(angle_y);

        let current_transformation =
            current_translation * y_rotation * x_rotation * center_transformation;

        for glyph_run in &self.glyph_runs {
            // let center_x: i32 = (glyph_run.metrics.width / 2) as _;
            // let center_y: i32 = ((glyph_run.metrics.size()).1 / 2) as _;

            let local_offset = (glyph_run.0.x.floor(), glyph_run.0.y.floor());
            let local_offset_matrix =
                Matrix4::from_translation((local_offset.0, local_offset.1, 0.0).into());

            let matrix = shell.pixel_matrix() * current_transformation * local_offset_matrix;

            // TODO: Should we use `Rc` for GlyphRuns, too, so that that the application can keep them stored.
            shapes.push(Shape::GlyphRun(matrix.into(), glyph_run.1.clone()));
        }

        (self.camera, shapes)
    }
}
