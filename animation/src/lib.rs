mod animated;
mod blended_animation;
mod interpolatable;
mod interpolation;
mod tickery;

pub use animated::*;
pub use blended_animation::*;
pub use interpolatable::*;
pub use interpolation::*;
pub use tickery::*;

mod time {
    #[cfg(not(target_arch = "wasm32"))]
    pub use std::time::Instant;
    #[cfg(target_arch = "wasm32")]
    pub use web_time::Instant;
}
