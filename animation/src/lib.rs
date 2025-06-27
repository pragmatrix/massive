mod blended_animation;
mod coordinator;
mod interpolatable;
mod interpolation;
mod timeline;

pub use blended_animation::*;
pub use coordinator::Coordinator;
pub use interpolatable::*;
pub use interpolation::*;
pub use timeline::*;

mod time {
    #[cfg(not(target_arch = "wasm32"))]
    pub use std::time::Instant;
    #[cfg(target_arch = "wasm32")]
    pub use web_time::Instant;
}
