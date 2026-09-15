//! Concrete shaping engine implementations.
//!
//! Each submodule wraps one shaping library and translates its layout output into the
//! engine-neutral model ([`crate::engine`]). The engines are selected at runtime via
//! [`crate::ShapingEngineKind`].

#[cfg(feature = "cosmic-text")]
mod cosmic_engine;
#[cfg(feature = "parley")]
mod parley_engine;

#[cfg(feature = "cosmic-text")]
pub use cosmic_engine::CosmicTextEngine;
#[cfg(feature = "parley")]
pub use parley_engine::ParleyEngine;
