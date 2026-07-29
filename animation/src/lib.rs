mod animated;
mod blended_animation;
mod animation_coordinator;
mod interpolatable;
mod interpolation;
mod movement_runtime;
mod time_scale;

pub use animated::*;
pub use blended_animation::*;
pub use animation_coordinator::*;
pub use interpolatable::*;
pub use interpolation::*;
pub use movement_runtime::*;
pub use time_scale::*;

mod time {
    #[cfg(not(target_arch = "wasm32"))]
    pub use std::time::Instant;
    #[cfg(target_arch = "wasm32")]
    pub use web_time::Instant;
}
