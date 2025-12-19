use std::{collections::VecDeque, io, time::Duration};

use anyhow::Result;
use cosmic_text::FontSystem;
use log::{debug, warn};
use logs::terminal::{self, color_schemes};
use termwiz::escape;
use tokio::{
    select,
    sync::mpsc::{self, UnboundedReceiver},
};
use tracing_subscriber::{
    EnvFilter, Layer, filter, fmt, layer::SubscriberExt, util::SubscriberInitExt,
};
use winit::{
    dpi::LogicalSize,
    event::{ElementState, KeyEvent, WindowEvent},
};

use massive_animation::{Animated, Interpolation};
use massive_geometry::Vector3;
use massive_scene::{Handle, Location, Transform, Visual};
use massive_shapes::Shape;
use massive_shell::{
    ApplicationContext, FontManager, Scene, ShellWindow,
    shell::{self, ShellEvent},
};

use shared::{
    application::{Application, UpdateResponse},
    attributed_text,
};

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

    shell::run(|ctx| logs(receiver, ctx))
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
    let fonts = FontManager::bare("en-US").with_font(shared::fonts::JETBRAINS_MONO);

    // Window

    let size = LogicalSize::new(1280., 800.).to_physical(ctx.primary_monitor_scale_factor());
    let window = ctx.new_window((size.width, size.height)).await?;

    let mut renderer = window.renderer().with_text(fonts.clone()).build().await?;

    let scene = ctx.new_scene();
    let mut logs = Logs::new(&scene, fonts);

    // Application

    loop {
        select! {
            Some(bytes) = receiver.recv() => {
                logs.add_line(&scene, &bytes);
                logs.update_layout()?;
                scene.render_to(&mut renderer)?;
            },

            Ok(event) = ctx.wait_for_shell_event() => {
                if logs.handle_shell_event(&event, &window) == UpdateResponse::Exit {
                    return Ok(())
                }
                renderer.resize_redraw(&event)?;
                scene.render_to(&mut renderer)?;
            }
        }
    }
}

struct Logs {
    fonts: FontManager,

    application: Application,

    content_transform: Handle<Transform>,

    content_width: u32,
    content_height: Animated<f64>,
    vertical_center: Animated<f64>,
    vertical_center_transform: Handle<Transform>,
    location: Handle<Location>,
    lines: VecDeque<LogLine>,
    next_line_top: f64,
}

impl Logs {
    fn new(scene: &Scene, fonts: FontManager) -> Self {
        let content_width = 1280;
        let application = Application::default();
        let current_transform = application.get_transform((content_width, content_width));
        let content_transform = scene.stage(current_transform);
        let content_location = scene.stage(Location::from(content_transform.clone()));

        let vertical_center = scene.animated(0.0);

        // We move up the lines by their top position.
        let vertical_center_transform = scene.stage(Transform::IDENTITY);

        // Final position for all lines (runs are y-translated, but only increasing).
        let location = scene.stage(Location {
            parent: Some(content_location),
            transform: vertical_center_transform.clone(),
        });

        let content_height = scene.animated(0.0);

        Self {
            fonts,
            application,
            content_transform,
            content_width,
            content_height,
            vertical_center,
            vertical_center_transform,
            location,
            lines: VecDeque::new(),
            next_line_top: 0.,
        }
    }

    fn add_line(&mut self, scene: &Scene, bytes: &[u8]) {
        let (glyph_runs, height) = {
            let mut font_system = self.fonts.lock();
            shape_log_line(bytes, self.next_line_top, &mut font_system)
        };

        let glyph_runs: Vec<Shape> = glyph_runs.into_iter().map(|run| run.into()).collect();

        let line = Visual::new(self.location.clone(), glyph_runs);
        let line = scene.stage(line);

        self.lines.push_back(LogLine {
            top: self.next_line_top,
            fader: scene.animation(0., 1., FADE_DURATION, Interpolation::CubicOut),
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
                        .animate(0., FADE_DURATION, Interpolation::CubicIn);
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

        self.vertical_center.animate(
            -top_line.top,
            VERTICAL_ALIGNMENT_DURATION,
            Interpolation::CubicOut,
        );

        let new_height = self.lines.len().min(MAX_LINES) as u32 * LINE_HEIGHT;
        // Final value should always a multiple of two so that we snap on the pixels when centering.
        // While a size animation runs, it's fine that we don't.
        assert!(new_height.is_multiple_of(2));
        self.content_height.animate(
            new_height as f64,
            VERTICAL_ALIGNMENT_DURATION,
            Interpolation::CubicOut,
        );
    }

    fn handle_shell_event(
        &mut self,
        shell_event: &ShellEvent,
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

            self.update_content_transform();
        }

        UpdateResponse::Continue
    }

    fn apply_animations(&mut self) {
        let v_center = self.vertical_center.value();
        dbg!(v_center);
        self.vertical_center_transform
            .update((0., v_center, 0.).into());

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

        self.update_content_transform();

        for line in &mut self.lines {
            line.apply_animations();
        }
    }

    fn update_content_transform(&mut self) {
        let new_transform = self
            .application
            .get_transform((self.content_width, self.content_height.value() as u32));
        self.content_transform.update_if_changed(new_transform);
    }
}

const LINE_HEIGHT: u32 = 40;

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
        LINE_HEIGHT as f32,
        Vector3::new(0., y, 0.),
    );
    (runs, height)
}

struct LogLine {
    top: f64,
    visual: Handle<Visual>,
    fader: Animated<f64>,
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
            v.shapes = v
                .shapes
                .iter()
                .cloned()
                .map(|mut shape| {
                    if let Shape::GlyphRun(ref mut glyph_run) = shape {
                        glyph_run.text_color.alpha = fading as f32;
                        glyph_run.translation.z = (1.0 - fading) * -Self::FADE_TRANSLATION;
                    }
                    shape
                })
                .collect::<Vec<_>>()
                .into()
        });
    }
}
