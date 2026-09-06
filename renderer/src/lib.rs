mod color_buffer;
mod glyph;
mod pods;
// mod quads;
mod builder;
mod config;
mod render_batches;
mod render_device;
mod render_geometry;
mod render_submission;
mod renderer;
mod scene;
mod shape_renderer;
mod size_buffer;
mod stats;
mod text_layer;
mod tools;
mod transactions;

pub use builder::*;
pub use color_buffer::*;
pub use config::*;
// FontManager and friends now live in massive-shapes, where shaping owns the contexts.
pub use massive_shapes::FontManager;
pub use render_device::*;
pub use render_geometry::{RenderGeometry, ViewProjections};
pub use render_submission::*;
pub use renderer::{PresentationMode, Renderer};
pub use size_buffer::*;
pub use transactions::*;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum RenderPacing {
    #[default]
    // Render as fast as possible to be able to represent input changes.
    Fast,
    // Render as smooth as possible so that animations are synced to the frame rate.
    Smooth,
}

pub trait RenderTarget {
    fn render(&mut self, submission: RenderSubmission) -> anyhow::Result<()>;
}
