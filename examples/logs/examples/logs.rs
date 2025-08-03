use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use cosmic_text::{fontdb, FontSystem};
use log::{debug, warn};
use termwiz::escape;
use tokio::{
    select,
    sync::mpsc::{self, UnboundedReceiver},
};
use tracing_subscriber::{
    filter, fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer,
};
use winit::{
    dpi::LogicalSize,
    event::{ElementState, KeyEvent, WindowEvent},
};

use massive_animation::{Interpolation, Timeline};
use massive_geometry::{Camera, Identity, Vector3};
use massive_scene::{Handle, Location, Matrix, Scene, Shape, Visual};
use massive_shell::{
    application_context::UpdateCycle,
    shell::{self, ShellEvent},
    ApplicationContext, ShellWindow,
};

use logs::terminal::{self, color_schemes};
use shared::{
    application::{Application, UpdateResponse},
    attributed_text,
};

const CANVAS_ID: &str = "massive-logs";

const FADE_DURATION: Duration = Duration::from_millis(400);
const VERTICAL_ALIGNMENT_DURATION: Duration = Duration::from_millis(400);

const MAX_LINES: usize = 32;

#[tokio::main]
async fn main() -> Result<()> {
    let (sender, receiver) = mpsc::unbounded_channel();

    let stdout_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .with_filter(EnvFilter::from_default_env());

    let info_only_layer = fmt::layer()
        .with_writer(move || -> Box<dyn io::Write> { Box::new(Sender(sender.clone())) })
        .with_filter(filter::LevelFilter::WARN);

    tracing_subscriber::registry()
        .with(stdout_layer)
        .with(info_only_layer)
        .init();

    shell::run(|ctx| logs(receiver, ctx)).await
}

struct Sender(mpsc::UnboundedSender<Vec<u8>>);

impl io::Write for Sender {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .send(buf.to_vec())
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

async fn logs(mut receiver: UnboundedReceiver<Vec<u8>>, mut ctx: ApplicationContext) -> Result<()> {
    let font_system = {
        let mut db = fontdb::Database::new();
        db.load_font_data(shared::fonts::JETBRAINS_MONO.to_vec());
        // Use an invariant locale.
        let fs = FontSystem::new_with_locale_and_db("en-US".into(), db);
        Arc::new(Mutex::new(fs))
    };

    // Window

    let window_size = LogicalSize::new(1280., 800.);

    let window = ctx.new_window(window_size, Some(CANVAS_ID)).await?;

    // Camera

    let camera = {
        let fovy: f64 = 45.0;
        let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();
        Camera::new((0.0, 0.0, camera_distance), (0.0, 0.0, 0.0))
    };

    let mut renderer = window
        .new_renderer(font_system.clone(), camera, window.inner_size())
        .await?;

    let scene = Scene::new();
    let mut logs = Logs::new(&mut ctx, &scene, font_system);

    // Application

    loop {
        select! {
            Some(bytes) = receiver.recv() => {
                let cycle = ctx.begin_update_cycle(&scene, &mut renderer, None)?;
                logs.add_line(&cycle, &bytes);
                logs.update_layout()?;
            },

            Ok(event) = ctx.wait_for_shell_event(&mut renderer) => {
                let _cycle = ctx.begin_update_cycle(&scene, &mut renderer, Some(&event))?;
                if logs.handle_shell_event(event, &window) == UpdateResponse::Exit {
                    return Ok(())
                }
            }
        }
    }
}

struct Logs {
    font_system: Arc<Mutex<FontSystem>>,

    application: Application,

    page_matrix: Handle<Matrix>,

    page_width: u32,
    page_height: Timeline<f64>,
    vertical_center: Timeline<f64>,
    vertical_center_matrix: Handle<Matrix>,
    location: Handle<Location>,
    lines: VecDeque<LogLine>,
    next_line_top: f64,
}

impl Logs {
    fn new(
        ctx: &mut ApplicationContext,
        scene: &Scene,
        font_system: Arc<Mutex<FontSystem>>,
    ) -> Self {
        let page_width = 1280u32;
        let application = Application::default();
        let current_matrix = application.matrix((page_width, page_width));
        let page_matrix = scene.stage(current_matrix);
        let page_location = scene.stage(Location::from(page_matrix.clone()));

        let vertical_center = ctx.timeline(0.0);

        // We move up the lines by their top position.
        let vertical_center_matrix = scene.stage(Matrix::identity());

        // Final position for all lines (runs are y-translated, but only increasing).
        let location = scene.stage(Location {
            parent: Some(page_location),
            matrix: vertical_center_matrix.clone(),
        });

        let page_height = ctx.timeline(0.0);

        Self {
            font_system,
            application,
            page_matrix,
            page_width,
            page_height,
            vertical_center,
            vertical_center_matrix,
            location,
            lines: VecDeque::new(),
            next_line_top: 0.,
        }
    }

    fn add_line(&mut self, cycle: &UpdateCycle, bytes: &[u8]) {
        let (glyph_runs, height) = {
            let mut font_system = self.font_system.lock().unwrap();

            shape_log_line(bytes, self.next_line_top, &mut font_system)
        };

        let glyph_runs: Vec<Shape> = glyph_runs.into_iter().map(|run| run.into()).collect();

        let line = Visual::new(self.location.clone(), glyph_runs);
        let line = cycle.scene().stage(line);

        self.lines.push_back(LogLine {
            top: self.next_line_top,
            fader: cycle.animation(0., 1., FADE_DURATION, Interpolation::CubicOut),
            visual: line,
            fading_out: false,
        });

        self.next_line_top += height;
    }

    fn update_layout(&mut self) -> Result<()> {
        // See if some lines need to be faded out.

        {
            let overhead_lines = self.lines.len().saturating_sub(MAX_LINES);

            for line in self.lines.iter_mut().take(overhead_lines) {
                if !line.fading_out {
                    line.fader
                        .animate_to(0., FADE_DURATION, Interpolation::CubicIn);
                    line.fading_out = true;
                }
            }
        }

        // Update page size.

        self.update_vertical_alignment();

        Ok(())
    }

    fn update_vertical_alignment(&mut self) {
        let top_line = self
            .lines
            .iter()
            .find(|l| !l.is_fading())
            .unwrap_or(self.lines.front().unwrap());

        self.vertical_center.animate_to(
            -top_line.top,
            VERTICAL_ALIGNMENT_DURATION,
            Interpolation::CubicOut,
        );

        let new_height = self.lines.len().min(MAX_LINES) as f32 * LINE_HEIGHT;
        self.page_height.animate_to(
            new_height as f64,
            VERTICAL_ALIGNMENT_DURATION,
            Interpolation::CubicOut,
        );
    }

    fn handle_shell_event(
        &mut self,
        shell_event: ShellEvent,
        window: &ShellWindow,
    ) -> UpdateResponse {
        if shell_event.apply_animations() {
            self.apply_animations();
            return UpdateResponse::Continue;
        }

        if let Some(window_event) = shell_event.window_event_for(window) {
            if let WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } = window_event
            {
                // Warning levels gets captured and forwarded to the application itself.
                warn!("{window_event:?}");
            }

            match self.application.update(window_event) {
                UpdateResponse::Exit => {
                    return UpdateResponse::Exit;
                }
                UpdateResponse::Continue => {}
            }

            self.update_page_matrix();
        }

        UpdateResponse::Continue
    }

    fn apply_animations(&mut self) {
        self.vertical_center_matrix.update(Matrix::from_translation(
            (0., self.vertical_center.value(), 0.).into(),
        ));

        // Remove all lines that finished fading out from top to bottom.

        let mut update_v_alignment = false;

        while let Some(line) = self.lines.front() {
            if line.fading_out && !line.fader.is_animating() {
                debug!("faded out at: {}", line.fader.value());
                self.lines.pop_front();
                update_v_alignment = true;
            } else {
                break;
            }
        }

        if update_v_alignment {
            self.update_vertical_alignment();
        }

        // DI: there is a director.action in update_page_matrix().
        self.update_page_matrix();

        for line in &mut self.lines {
            line.apply_animations();
        }
    }

    fn update_page_matrix(&mut self) {
        // DI: This check has to be done in the renderer and the renderer has to decide when
        // it needs to redraw.
        let new_matrix = self
            .application
            .matrix((self.page_width, self.page_height.value() as u32));
        self.page_matrix.update_if_changed(new_matrix);
    }
}

const LINE_HEIGHT: f32 = 40.;

fn shape_log_line(
    bytes: &[u8],
    y: f64,
    font_system: &mut FontSystem,
) -> (Vec<massive_shapes::GlyphRun>, f64) {
    // OO: Share Parser between runs.
    let mut parser = escape::parser::Parser::new();
    let parsed = parser.parse_as_vec(bytes);

    // OO: Share Processor between runs.
    let mut processor = terminal::TextAttributor::new(color_schemes::light::PAPER);
    for action in parsed {
        processor.process(action)
    }

    let (text, attributes) = processor.into_text_and_attribute_ranges();

    let font_size = 32.;

    let (runs, height) = attributed_text::shape_text(
        font_system,
        &text,
        &attributes,
        font_size,
        LINE_HEIGHT,
        Vector3::new(0., y, 0.),
    );
    (runs, height)
}

struct LogLine {
    top: f64,
    visual: Handle<Visual>,
    fader: Timeline<f64>,
    fading_out: bool,
}

impl LogLine {
    const FADE_TRANSLATION: f64 = 256.0;

    pub fn is_fading(&self) -> bool {
        self.fader.is_animating()
    }

    pub fn apply_animations(&mut self) {
        if !self.fader.is_animating() {
            return;
        }

        let fading = self.fader.value();

        self.visual.update_with(|v| {
            for shape in &mut v.shapes {
                if let Shape::GlyphRun(glyph_run) = shape {
                    glyph_run.text_color.alpha = fading as f32;
                    glyph_run.translation.z = (1.0 - fading) * -Self::FADE_TRANSLATION;
                }
            }
        });
    }
}
