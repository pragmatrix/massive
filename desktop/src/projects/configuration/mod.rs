//! A project configuration represents a plane in space that has a specific layout of tiles and
//! groups and acts as a launching space for new applications.

mod toml_reader;
mod types;

pub use toml_reader::load_configuration;
pub use types::*;
