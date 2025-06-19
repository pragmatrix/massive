mod blended_animation;
mod interpolatable;
mod interpolation;
mod tickery;
mod timeline;

pub use blended_animation::*;
pub use interpolatable::*;
pub use interpolation::*;
pub use tickery::*;
pub use timeline::*;

mod time {
    #[cfg(not(target_arch = "wasm32"))]
    pub use std::time::Instant;
    #[cfg(target_arch = "wasm32")]
    pub use web_time::Instant;
}
