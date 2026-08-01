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

use massive_animation::{Animated, Interpolation, Movement, MovementRuntime};
use massive_applications::ApplicationEvent;
use massive_geometry::Vector3;
use massive_scene::{At, Handle, Location, Object, ToLocation, Transform};
use massive_shapes::Shape;
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
    let view_id = window.view_id();

    let mut renderer = window.renderer().with_text(fonts.clone()).build().await?;

    let scene = ctx.new_scene();
    let mut logs = Logs::new(&scene, ctx.movement_runtime(), fonts);

    // Application

    loop {
        // Resolve the wakeup first, so that the borrow of `ctx` ends before the frame is built.
        let wakeup = select! {
            Some(bytes) = receiver.recv() => Wakeup::Line(bytes),
            events = ctx.wait_for_events() => Wakeup::Events(events?),
        };

        let mut frame = ctx.frame(&scene);

        match wakeup {
            Wakeup::Line(bytes) => {
                logs.add_line(&mut frame, &bytes);
                logs.update_layout()?;
            }
            Wakeup::Events(events) => {
                for event in events {
                    match event {
                        ApplicationEvent::View(event_view_id, view_event) if event_view_id == view_id => {
                            if logs.handle_window_event(&view_event) == UpdateResponse::Exit {
                                return Ok(());
                            }
                            renderer.resize_redraw(&view_event)?;
                        }
                        ApplicationEvent::View(..) | ApplicationEvent::Shutdown(_) => {}
                        ApplicationEvent::ApplyAnimations(_) => {}
                        ApplicationEvent::Custom(LogEvent::FadeCompleted(line_id)) => {
                            logs.finish_fade_out(line_id);
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

    application_transform: Handle<Transform>,
    layout: Movement<LayoutMovement>,
    location: Handle<Location>,
    lines: VecDeque<LogLine>,
    next_line_top: f64,
    next_line_id: usize,
}

impl Logs {
    fn new(scene: &Scene, movement: &mut MovementRuntime, fonts: FontManager) -> Self {
        let content_width = 1280;
        let application = Application::default();

        let application_transform = application.get_transform((0, 0)).enter(scene);
        let application_location = application_transform.to_location().enter(scene);

        // Keep interaction transforms separate so the movement owns only animated centering.
        let content_transform = Transform::from_xy(-(content_width as f64) / 2., 0.).enter(scene);
        let content_location = content_transform
            .to_location()
            .relative_to(&application_location)
            .enter(scene);

        let vertical_center_transform = Transform::IDENTITY.enter(scene);
        let location = vertical_center_transform
            .to_location()
            .relative_to(&content_location)
            .enter(scene);

        let layout = movement
            .movement(
                LayoutMovement {
                    content_height: 0.0.into(),
                    vertical_center: 0.0.into(),
                },
                move |layout, context| {
                    let content_height = *layout.content_height.value(context);
                    content_transform.update_if_changed(Transform::from_xy(
                        -(content_width as f64) / 2.,
                        -content_height / 2.,
                    ));

                    let vertical_center = *layout.vertical_center.value(context);
                    vertical_center_transform.update_if_changed((0., vertical_center, 0.).into());
                },
            )
            .mount();

        Self {
            fonts,
            application,
            application_transform,
            layout,
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

        let line = glyph_runs
            .at(&self.location)
            .with_decal_order(0)
            .enter(frame.scene());

        let line_id = self.next_line_id;
        let fader: Animated<_> = 0.0.into();
        let fader = frame
            .movement(fader, move |fader, context| {
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
            })
            .completion_event(move || LogEvent::FadeCompleted(line_id))
            .mount();
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

    fn update_layout(&mut self) -> Result<()> {
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

        self.update_vertical_alignment();

        Ok(())
    }

    fn handle_window_event(&mut self, window_event: &WindowEvent) -> UpdateResponse {
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

        self.application_transform
            .update_if_changed(self.application.get_transform((0, 0)));

        UpdateResponse::Continue
    }

    fn finish_fade_out(&mut self, line_id: usize) {
        let Some(position) = self
            .lines
            .iter()
            .position(|line| line.id == line_id && line.fading_out)
        else {
            return;
        };

        self.lines.remove(position);
        debug!("faded out");

        // Fading lines remain in layout until removal, then start a successor transition.
        self.update_vertical_alignment();
    }

    fn update_vertical_alignment(&mut self) {
        let top_line = self
            .lines
            .iter()
            .find(|l| !l.is_fading())
            .unwrap_or(self.lines.front().unwrap());
        let top_line_top = top_line.top;

        let new_height = self.lines.len().min(MAX_LINES) as u32 * LINE_HEIGHT;
        // Final value should always a multiple of two so that we snap on the pixels when centering.
        // While a size animation runs, it's fine that we don't.
        assert!(new_height.is_multiple_of(2));
        self.layout.modify(move |layout, context| {
            layout.vertical_center.animate(
                context,
                -top_line_top,
                VERTICAL_ALIGNMENT_DURATION,
                Interpolation::CubicOut,
            );
            layout.content_height.animate(
                context,
                new_height as f64,
                VERTICAL_ALIGNMENT_DURATION,
                Interpolation::CubicOut,
            );
        });
    }
}

#[derive(Debug)]
struct LayoutMovement {
    content_height: Animated<f64>,
    vertical_center: Animated<f64>,
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
