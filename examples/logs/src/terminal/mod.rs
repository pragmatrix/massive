//! This module contains types and color schemes useful to process colored terminal output.
//! source: <https://github.com/pragmatrix/emergent>

mod color;
pub mod color_schemes;
mod config;
mod named_color;

pub use color::Rgb;
pub use config::AnsiColors;
pub use named_color::NamedColor;
