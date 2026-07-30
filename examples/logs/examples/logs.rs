use std::collections::VecDeque;
use std::io;
use std::time::Duration;

use anyhow::Result;
use log::{debug, warn};
use logs::terminal;
use logs::terminal::color_schemes;
use tokio::select;
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tracing_subscriber::filter;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use cosmic_text::FontSystem;
use termwiz::escape;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};

use massive_animation::{Animated, AnimationContext, Interpolation, Movement};
use massive_geometry::Vector3;
use massive_scene::{At, Handle, Location, Object, ToLocation, Transform};
use massive_shapes::Shape;
use massive_shell::application_context::ApplicationEvent;
use massive_shell::shell;
use massive_shell::{ApplicationContext, FontManager, Frame, Scene};

use shared::application::{Application, UpdateResponse};
use shared::attributed_text;

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
        // Resolve the wakeup first, so that the borrow of `ctx` ends before the frame is built.
        let wakeup = select! {
            Some(bytes) = receiver.recv() => Wakeup::Line(bytes),
            Ok(events) = ctx.wait_for_events::<LogEvent>() => Wakeup::Events(events),
        };

        let mut frame = ctx.frame(&scene);

        match wakeup {
            Wakeup::Line(bytes) => {
                logs.add_line(&mut frame, &bytes);
                logs.update_layout(&mut frame)?;
            }
            Wakeup::Events(events) => {
                for event in events {
                    match event {
                        ApplicationEvent::Window(_, window_event) => {
                            if logs.handle_window_event(&mut frame, &window_event)
                                == UpdateResponse::Exit
                            {
                                return Ok(());
                            }
                            renderer.resize_redraw(&window_event)?;
                        }
                        ApplicationEvent::ApplyAnimations(_) => {
                            logs.apply_animations(&mut frame);
                        }
                        ApplicationEvent::Custom(LogEvent::FadeCompleted(line_id)) => {
                            logs.finish_fade_out(line_id, &mut frame);
                        }
                    }
                }
            }
        }

        frame.render_to(&mut renderer)?;
    }
}

enum Wakeup {
    Line(Vec<u8>),
    Events(Vec<ApplicationEvent<LogEvent>>),
}

#[derive(Debug)]
enum LogEvent {
    FadeCompleted(usize),
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
    next_line_id: usize,
}

impl Logs {
    fn new(scene: &Scene, fonts: FontManager) -> Self {
        let content_width = 1280;
        let application = Application::default();
        let current_transform = application.get_transform((content_width, content_width));
        let content_transform = current_transform.enter(scene);
        let content_location = content_transform.to_location().enter(scene);

        let vertical_center = 0.0.into();

        // We move up the lines by their top position.
        let vertical_center_transform = Transform::IDENTITY.enter(scene);

        // Final position for all lines (runs are y-translated, but only increasing).
        let location = vertical_center_transform
            .to_location()
            .relative_to(&content_location)
            .enter(scene);

        let content_height = 0.0.into();

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
            next_line_id: 0,
        }
    }

    fn add_line(&mut self, frame: &mut Frame, bytes: &[u8]) {
        let (glyph_runs, height) = {
            let mut font_system = self.fonts.lock();
            shape_log_line(bytes, self.next_line_top, &mut font_system)
        };

        let glyph_runs: Vec<Shape> = glyph_runs.into_iter().map(|run| run.into()).collect();

        let line = glyph_runs.at(&self.location).enter(frame.scene());

        let line_id = self.next_line_id;
        let fader: Animated<_> = 0.0.into();
        let fader = frame.movement(
            fader,
            move |fader, context| {
                assert!(
                    fader.is_animating(),
                    "Internal error: animation state is not in sync with the context"
                );
                let fading = *fader.value(context);
                line.update_with(|visual| {
                    visual.shapes = visual
                        .shapes
                        .iter()
                        .cloned()
                        .map(|mut shape| {
                            if let Shape::GlyphRun(ref mut glyph_run) = shape {
                                glyph_run.text_color.alpha = fading as f32;
                                glyph_run.translation.z =
                                    (1.0 - fading) * -LogLine::FADE_TRANSLATION;
                            }
                            shape
                        })
                        .collect::<Vec<_>>()
                        .into()
                });
            },
            move || LogEvent::FadeCompleted(line_id),
        );
        fader.modify(|fader, context| {
            fader.animate(context, 1.0, FADE_DURATION, Interpolation::CubicOut);
        });
        self.lines.push_back(LogLine {
            id: line_id,
            top: self.next_line_top,
            fader,
            fading_out: false,
        });

        self.next_line_top += height;
        self.next_line_id += 1;
    }

    fn update_layout(&mut self, context: &mut dyn AnimationContext) -> Result<()> {
        // See if some lines need to be faded out.

        {
            let overhead_lines = self.lines.len().saturating_sub(MAX_LINES);

            for line in self.lines.iter_mut().take(overhead_lines) {
                if !line.fading_out {
                    line.fader.modify(|fader, context| {
                        fader.animate(context, 0., FADE_DURATION, Interpolation::CubicIn);
                    });
                    line.fading_out = true;
                }
            }
        }

        // Update page size.

        self.update_vertical_alignment(context);

        Ok(())
    }

    fn update_vertical_alignment(&mut self, context: &mut dyn AnimationContext) {
        let top_line = self
            .lines
            .iter()
            .find(|l| !l.is_fading())
            .unwrap_or(self.lines.front().unwrap());

        self.vertical_center.animate(
            context,
            -top_line.top,
            VERTICAL_ALIGNMENT_DURATION,
            Interpolation::CubicOut,
        );

        let new_height = self.lines.len().min(MAX_LINES) as u32 * LINE_HEIGHT;
        // Final value should always a multiple of two so that we snap on the pixels when centering.
        // While a size animation runs, it's fine that we don't.
        assert!(new_height.is_multiple_of(2));
        self.content_height.animate(
            context,
            new_height as f64,
            VERTICAL_ALIGNMENT_DURATION,
            Interpolation::CubicOut,
        );
    }

    fn handle_window_event(
        &mut self,
        context: &mut impl AnimationContext,
        window_event: &WindowEvent,
    ) -> UpdateResponse {
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

        self.update_content_transform(context);

        UpdateResponse::Continue
    }

    fn apply_animations(&mut self, context: &mut impl AnimationContext) {
        let v_center = *self.vertical_center.value(context);
        self.vertical_center_transform
            .update((0., v_center, 0.).into());

        self.update_content_transform(context);
    }

    fn finish_fade_out(&mut self, line_id: usize, context: &mut impl AnimationContext) {
        let Some(position) = self
            .lines
            .iter()
            .position(|line| line.id == line_id && line.fading_out)
        else {
            return;
        };

        self.lines.remove(position);
        debug!("faded out");

        self.update_vertical_alignment(context);
        self.update_content_transform(context);
    }

    fn update_content_transform(&mut self, context: &impl AnimationContext) {
        let content_height = *self.content_height.value(context);
        let new_transform = self
            .application
            .get_transform((self.content_width, content_height as u32));
        self.content_transform.update_if_changed(new_transform);
    }
}

const LINE_HEIGHT: u32 = 40;

fn shape_log_line(
    bytes: &[u8],
    y: f64,
    font_system: &mut FontSystem,
) -> (Vec<massive_shapes::GlyphRun>, f64) {
    // Optimization: Share Parser between runs.
    let mut parser = escape::parser::Parser::new();
    let parsed = parser.parse_as_vec(bytes);

    // Optimization: Share Processor between runs.
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
    id: usize,
    top: f64,
    fader: Movement<Animated<f64>>,
    fading_out: bool,
}

impl LogLine {
    const FADE_TRANSLATION: f64 = 256.0;

    pub fn is_fading(&self) -> bool {
        self.fading_out
    }
}
