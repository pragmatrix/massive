//! Concrete shaping engine implementations.
//!
//! Each submodule wraps one shaping library and translates its layout output into the
//! engine-neutral model ([`crate::engine`]). The engines are selected at runtime via
//! [`crate::ShapingEngineKind`].

mod cosmic_engine;
mod parley_engine;

pub use cosmic_engine::CosmicTextEngine;
pub use parley_engine::ParleyEngine;
