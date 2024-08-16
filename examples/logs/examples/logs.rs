use std::{
    collections::VecDeque,
    io, iter,
    ops::Range,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use cosmic_text::{fontdb, FontSystem};
use log::{debug, warn};
use massive_animation::{Interpolation, Timeline};
use termwiz::{
    cell::Intensity,
    color::ColorSpec,
    escape::{self, csi::Sgr, Action, ControlCode, CSI},
};
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

use logs::terminal::{color_schemes, Rgb};
use massive_geometry::{Camera, Color, Identity, Vector3};
use massive_scene::{Director, Handle, Location, Matrix, Shape, Visual};
use massive_shapes::TextWeight;
use massive_shell::{
    shell::{self, ShellEvent},
    ApplicationContext, ShellWindow,
};
use shared::{
    application::{Application, UpdateResponse},
    attributed_text::{self, TextAttribute},
};

const CANVAS_ID: &str = "massive-logs";

const FADE_DURATION: Duration = Duration::from_millis(400);
const VERTICAL_ALIGNMENT_DURATION: Duration = Duration::from_millis(400);

const MAX_LINES: usize = 32;

fn main() -> Result<()> {
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

    shared::main(|| async_main(receiver))
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

async fn async_main(receiver: UnboundedReceiver<Vec<u8>>) -> Result<()> {
    shell::run(|ctx| logs(receiver, ctx)).await
}

async fn logs(mut receiver: UnboundedReceiver<Vec<u8>>, mut ctx: ApplicationContext) -> Result<()> {
    let font_system = {
        let mut db = fontdb::Database::new();
        db.load_font_data(shared::fonts::JETBRAINS_MONO.to_vec());
        // Use an invariant locale.
        FontSystem::new_with_locale_and_db("en-US".into(), db)
    };

    // Window

    let window_size = LogicalSize::new(1280., 800.);

    let window = ctx.new_window(window_size, Some(CANVAS_ID))?;

    // Camera

    let camera = {
        let fovy: f64 = 45.0;
        let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();
        Camera::new((0.0, 0.0, camera_distance), (0.0, 0.0, 0.0))
    };

    let font_system = Arc::new(Mutex::new(font_system));

    let (mut renderer, director) = window
        .new_renderer(font_system.clone(), camera, window.inner_size())
        .await?;

    let mut logs = Logs::new(&mut ctx, font_system, director);

    // Application

    loop {
        select! {
            Some(bytes) = receiver.recv() => {
                logs.add_line(&mut ctx, &bytes)?;
            },

            Ok(event) = ctx.wait_for_event() => {
                if let Some(window_event) = event.window_event_for(&window) {
                    renderer.handle_window_event(window_event)?;
                }

                let r = logs.handle_event(event, &window)?;
                if r == UpdateResponse::Exit {
                    return Ok(());
                }
            }
        }
    }
}

struct Logs {
    font_system: Arc<Mutex<FontSystem>>,

    application: Application,

    current_matrix: Matrix,
    page_matrix: Handle<Matrix>,

    page_width: u32,
    page_height: Timeline<f64>,
    vertical_center: Timeline<f64>,
    vertical_center_matrix: Handle<Matrix>,
    location: Handle<Location>,
    director: Director,
    lines: VecDeque<LogLine>,
    y: f64,
}

impl Logs {
    fn new(
        ctx: &mut ApplicationContext,
        font_system: Arc<Mutex<FontSystem>>,
        mut director: Director,
    ) -> Self {
        let page_height = 1;
        let page_width = 1280u32;
        let application = Application::default();
        let current_matrix = application.matrix((page_width, page_width));
        let page_matrix = director.stage(current_matrix);
        let page_location = director.stage(Location::from(page_matrix.clone()));

        let vertical_center = ctx.timeline(0.0);

        // We move up the lines by their top position.
        let vertical_center_matrix = director.stage(Matrix::identity());

        // Final position for all lines (runs are y-translated, but only increasing).
        let location = director.stage(Location {
            parent: Some(page_location),
            matrix: vertical_center_matrix.clone(),
        });

        let page_height = ctx.timeline(page_height as f64);

        Self {
            font_system,
            application,
            current_matrix,
            page_matrix,
            page_width,
            page_height,
            vertical_center,
            vertical_center_matrix,
            location,
            director,
            lines: VecDeque::new(),
            y: 0.,
        }
    }

    fn add_line(&mut self, ctx: &mut ApplicationContext, bytes: &[u8]) -> Result<()> {
        let (new_runs, height) = {
            let mut font_system = self.font_system.lock().unwrap();

            shape_log_line(bytes, self.y, &mut font_system)
        };

        let glyph_runs: Vec<Shape> = new_runs.into_iter().map(|run| run.into()).collect();

        let glyph_runs_visual = glyph_runs.clone();

        let visual = Visual::new(self.location.clone(), glyph_runs_visual);

        let line = self.director.stage(visual);

        self.lines.push_back(LogLine {
            y: self.y,
            height,
            fader: ctx.animation(0., 1., FADE_DURATION, Interpolation::CubicOut),
            glyph_runs,
            visual_handle: line,
            fading_out: false,
        });

        // See if some lines need to be faded out.

        {
            let overhead_lines = if self.lines.len() > MAX_LINES {
                self.lines.len() - MAX_LINES
            } else {
                0
            };

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

        self.director.action()?;

        self.y += height;

        Ok(())
    }

    fn handle_event(
        &mut self,
        shell_event: ShellEvent,
        window: &ShellWindow,
    ) -> Result<UpdateResponse> {
        if shell_event.apply_animations() {
            self.apply_animations()?;
            return Ok(UpdateResponse::Continue);
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
                warn!("{:?}", window_event);
            }

            match self.application.update(window_event) {
                UpdateResponse::Exit => return Ok(UpdateResponse::Exit),
                UpdateResponse::Continue => {}
            }

            self.update_page_matrix()?;
        }

        Ok(UpdateResponse::Continue)
    }

    fn apply_animations(&mut self) -> Result<()> {
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
        self.update_page_matrix()?;

        for line in &mut self.lines {
            line.apply_animations(&self.location)
        }

        self.director.action()
    }

    fn update_vertical_alignment(&mut self) {
        let top_line = self.lines.front().unwrap();

        self.vertical_center.animate_to(
            -top_line.y,
            VERTICAL_ALIGNMENT_DURATION,
            Interpolation::CubicOut,
        );

        let last_line = self.lines.back().unwrap();
        let new_height = last_line.y + last_line.height - top_line.y;
        self.page_height.animate_to(
            new_height,
            VERTICAL_ALIGNMENT_DURATION,
            Interpolation::CubicOut,
        );
    }

    fn update_page_matrix(&mut self) -> Result<()> {
        // DI: This check has to be done in the renderer and the renderer has to decide when
        // it needs to redraw.
        //
        // OO: Or, we introduce another handle type that stores the matrix locally and
        // compares it _before_ uploading.
        let new_matrix = self
            .application
            .matrix((self.page_width, self.page_height.value() as u32));
        if new_matrix != self.current_matrix {
            self.page_matrix.update(new_matrix);
            self.current_matrix = new_matrix;
            self.director.action()?;
        }
        Ok(())
    }
}

fn shape_log_line(
    bytes: &[u8],
    y: f64,
    font_system: &mut FontSystem,
) -> (Vec<massive_shapes::GlyphRun>, f64) {
    // OO: Share Parser between runs.
    let mut parser = escape::parser::Parser::new();
    let parsed = parser.parse_as_vec(bytes);

    // OO: Share Processor between runs.
    let mut processor = Processor::new(color_schemes::light::PAPER);
    for action in parsed {
        processor.process(action)
    }

    let (text, attributes) = processor.into_text_and_attribute_ranges();

    let font_size = 32.;
    let line_height = 40.;

    let (runs, height) = attributed_text::shape_text(
        font_system,
        &text,
        &attributes,
        font_size,
        line_height,
        Vector3::new(0., y, 0.),
    );
    (runs, height)
}

struct LogLine {
    y: f64,
    height: f64,
    // Stored here, because we need to change opacity.
    //
    // OO: Just provide opacity somehow as a property, or at least introduce a Handle<T> that stores
    // a local backup?
    glyph_runs: Vec<Shape>,
    visual_handle: Handle<Visual>,
    fader: Timeline<f64>,
    fading_out: bool,
}

impl LogLine {
    const FADE_TRANSLATION: f64 = 256.0;

    pub fn apply_animations(&mut self, location: &Handle<Location>) {
        if !self.fader.is_animating() {
            return;
        }

        let fading = self.fader.value();

        for shape in &mut self.glyph_runs {
            if let Shape::GlyphRun(glyph_run) = shape {
                glyph_run.text_color.alpha = fading as f32;
                glyph_run.translation.z = (1.0 - fading) * -Self::FADE_TRANSLATION;
            }
        }

        // OO: Avoid excessive cloning.
        self.visual_handle
            .update(Visual::new(location.clone(), self.glyph_runs.clone()));
    }
}

#[derive(Debug)]
struct Processor {
    default: Attributes,
    current: Attributes,
    color_scheme: color_schemes::Scheme,
    text: String,
    text_attributes: Vec<Attributes>,
}

#[derive(Debug, Copy, Clone, PartialEq)]
struct Attributes {
    pub foreground_color: Color,
    pub bold: bool,
}

impl Processor {
    pub fn new(color_scheme: color_schemes::Scheme) -> Self {
        let default_attributes = Attributes {
            foreground_color: rgb_to_color(color_scheme.primary.foreground),
            bold: false,
        };

        Self {
            default: default_attributes,
            current: default_attributes,
            color_scheme,
            text: String::new(),
            // TODO: Not quite efficient storing the attributes for each u8 inside a string.
            text_attributes: Vec::new(),
        }
    }

    pub fn into_text_and_attribute_ranges(self) -> (String, Vec<TextAttribute>) {
        // TODO: this is something like a slicetools candidate. AFAI(and ChatGPT)K all solutions to
        // this problem are either inefficient (generate intermediate Vecs) or hard to read.

        let mut ranges: Vec<TextAttribute> = Vec::new();

        if self.text_attributes.is_empty() {
            return (self.text, Vec::new());
        }

        let mut current_start = 0;

        for i in 1..self.text_attributes.len() {
            let prev = &self.text_attributes[i - 1];
            if *prev != self.text_attributes[i] {
                ranges.push(ta(current_start..i, prev));
                current_start = i;
            }
        }

        ranges.push(ta(
            current_start..self.text_attributes.len(),
            &self.text_attributes[current_start],
        ));

        return (self.text, ranges);

        fn ta(range: Range<usize>, attr: &Attributes) -> TextAttribute {
            TextAttribute {
                range,
                color: attr.foreground_color,
                weight: if attr.bold {
                    TextWeight::BOLD
                } else {
                    TextWeight::NORMAL
                },
            }
        }
    }

    pub fn process(&mut self, action: escape::Action) {
        match action {
            Action::Print(ch) => {
                self.text.push(ch);
                self.text_attributes.push(self.current)
            }
            Action::PrintString(string) => {
                self.text.push_str(&string);
                self.text_attributes
                    .extend(iter::repeat_n(self.current, string.len()))
            }
            Action::Control(control) => match control {
                ControlCode::Null => {}
                ControlCode::StartOfHeading => {}
                ControlCode::StartOfText => {}
                ControlCode::EndOfText => {}
                ControlCode::EndOfTransmission => {}
                ControlCode::Enquiry => {}
                ControlCode::Acknowledge => {}
                ControlCode::Bell => {}
                ControlCode::Backspace => {}
                ControlCode::HorizontalTab => {}
                ControlCode::LineFeed => {
                    self.text.push('\n');
                    self.text_attributes.push(self.current);
                }
                ControlCode::VerticalTab => {}
                ControlCode::FormFeed => {}
                ControlCode::CarriageReturn => {
                    self.text.push('\r');
                    self.text_attributes.push(self.current);
                }
                ControlCode::ShiftOut => {}
                ControlCode::ShiftIn => {}
                ControlCode::DataLinkEscape => {}
                ControlCode::DeviceControlOne => {}
                ControlCode::DeviceControlTwo => {}
                ControlCode::DeviceControlThree => {}
                ControlCode::DeviceControlFour => {}
                ControlCode::NegativeAcknowledge => {}
                ControlCode::SynchronousIdle => {}
                ControlCode::EndOfTransmissionBlock => {}
                ControlCode::Cancel => {}
                ControlCode::EndOfMedium => {}
                ControlCode::Substitute => {}
                ControlCode::Escape => {}
                ControlCode::FileSeparator => {}
                ControlCode::GroupSeparator => {}
                ControlCode::RecordSeparator => {}
                ControlCode::UnitSeparator => {}
                ControlCode::BPH => {}
                ControlCode::NBH => {}
                ControlCode::IND => {}
                ControlCode::NEL => {}
                ControlCode::SSA => {}
                ControlCode::ESA => {}
                ControlCode::HTS => {}
                ControlCode::HTJ => {}
                ControlCode::VTS => {}
                ControlCode::PLD => {}
                ControlCode::PLU => {}
                ControlCode::RI => {}
                ControlCode::SS2 => {}
                ControlCode::SS3 => {}
                ControlCode::DCS => {}
                ControlCode::PU1 => {}
                ControlCode::PU2 => {}
                ControlCode::STS => {}
                ControlCode::CCH => {}
                ControlCode::MW => {}
                ControlCode::SPA => {}
                ControlCode::EPA => {}
                ControlCode::SOS => {}
                ControlCode::SCI => {}
                ControlCode::CSI => {}
                ControlCode::ST => {}
                ControlCode::OSC => {}
                ControlCode::PM => {}
                ControlCode::APC => {}
            },
            Action::DeviceControl(_) => {}
            Action::OperatingSystemCommand(_) => {}
            Action::CSI(csi) => match csi {
                CSI::Sgr(sgr) => match sgr {
                    Sgr::Reset => self.current = self.default,
                    Sgr::Intensity(intensity) => match intensity {
                        Intensity::Normal => self.current.bold = false,
                        Intensity::Bold => self.current.bold = true,
                        Intensity::Half => {}
                    },
                    Sgr::Underline(_) => {}
                    Sgr::UnderlineColor(_) => {}
                    Sgr::Blink(_) => {}
                    Sgr::Italic(_) => {}
                    Sgr::Inverse(_) => {}
                    Sgr::Invisible(_) => {}
                    Sgr::StrikeThrough(_) => {}
                    Sgr::Font(_) => {}
                    Sgr::Foreground(foreground) => match foreground {
                        ColorSpec::Default => {
                            self.current.foreground_color = self.default.foreground_color
                        }
                        ColorSpec::PaletteIndex(index) => {
                            // TODO: this panics if the index is out of range.
                            let rgb = if index > 7 {
                                self.color_scheme.bright[(index - 8) as _]
                            } else {
                                self.color_scheme.normal[index as _]
                            };

                            self.current.foreground_color = rgb_to_color(rgb);
                        }
                        ColorSpec::TrueColor(_) => {}
                    },
                    Sgr::Background(_) => {}
                    Sgr::Overline(_) => {}
                    Sgr::VerticalAlign(_) => {}
                },
                CSI::Cursor(_) => {}
                CSI::Edit(_) => {}
                CSI::Mode(_) => {}
                CSI::Device(_) => {}
                CSI::Mouse(_) => {}
                CSI::Window(_) => {}
                CSI::Keyboard(_) => {}
                CSI::SelectCharacterPath(_, _) => {}
                CSI::Unspecified(_) => {}
            },
            Action::Esc(_) => {}
            Action::Sixel(_) => {}
            Action::XtGetTcap(_) => {}
            Action::KittyImage(_) => {}
        }
    }
}

fn rgb_to_color(value: Rgb) -> Color {
    (value.r, value.g, value.b).into()
}
