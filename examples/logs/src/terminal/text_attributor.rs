use std::{iter, ops::Range};

use termwiz::{
    cell::Intensity,
    color::ColorSpec,
    escape::{self, csi::Sgr, Action, ControlCode, CSI},
};

use crate::terminal::{color_schemes, Rgb};
use massive_geometry::Color;
use massive_shapes::TextWeight;
use shared::attributed_text::TextAttribute;

#[derive(Debug)]
pub struct TextAttributor {
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

impl TextAttributor {
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
