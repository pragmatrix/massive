use std::{
    io, iter,
    ops::Range,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use cosmic_text::{fontdb, FontSystem};
use env_logger::{Builder, Target, WriteStyle};
use log::error;
use termwiz::{
    cell::Intensity,
    color::ColorSpec,
    escape::{self, csi::Sgr, Action, ControlCode, CSI},
};
use tokio::{
    select,
    sync::mpsc::{self, UnboundedReceiver},
};
use winit::dpi::LogicalSize;

use logs::terminal::{color_schemes, Rgb};
use massive_geometry::{Camera, Color};
use massive_scene::PositionedShape;
use massive_shapes::TextWeight;
use massive_shell::{shell, ApplicationContext};
use shared::{
    application::{Application, UpdateResponse},
    attributed_text::{self, TextAttribute},
};

const CANVAS_ID: &str = "massive-logs";

fn main() -> Result<()> {
    let (sender, receiver) = mpsc::unbounded_channel();

    Builder::default()
        .filter(Some("massive_shell"), log::LevelFilter::Info)
        .write_style(WriteStyle::Always)
        .target(Target::Pipe(Box::new(Sender(sender))))
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
    error!("TEST");

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

    let (mut renderer, mut director) = window
        .new_renderer(font_system.clone(), camera, window.inner_size())
        .await?;

    // Application

    let page_size = (1280u32, 800);
    let mut application = Application::new(page_size);
    let mut current_matrix = application.matrix();
    let matrix_handle = director.cast(current_matrix);

    // Hold the positioned shapes in this context, otherwise they will disappear.
    let mut positioned_shapes = Vec::new();

    loop {
        select! {
            Some(bytes) = receiver
            .recv() => {
                let (new_runs, _height) = {
                    let mut font_system = font_system.lock().unwrap();
                    shape_log_line(&bytes, &mut font_system)
                };

                positioned_shapes.extend(
                    new_runs.into_iter().map(|run| director.cast(PositionedShape::new(matrix_handle.clone(), run)))
                );
                director.action()?;

            },

            Ok(window_event) = ctx.wait_for_event(&mut renderer) => {
                match application.update(window_event) {
                    UpdateResponse::Exit => return Ok(()),
                    UpdateResponse::Continue => {}
                }

                // DI: This check has to be done in the renderer and the renderer has to decide when it
                // needs to redraw.
                let new_matrix = application.matrix();
                if new_matrix != current_matrix {
                    matrix_handle.update(new_matrix);
                    current_matrix = new_matrix;
                    director.action()?;
                }
            }
        }
    }
}

fn shape_log_line(
    bytes: &[u8],
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

    let (runs, height) =
        attributed_text::shape_text(font_system, &text, &attributes, font_size, line_height);
    (runs, height)
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
                    .extend(iter::repeat(self.current).take(string.len()))
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
