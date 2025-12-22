mod color_buffer;
mod glyph;
mod pods;
// mod quads;
mod builder;
mod config;
mod font_manager;
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
pub use font_manager::*;
pub use render_device::*;
pub use render_geometry::RenderGeometry;
pub use render_submission::*;
pub use renderer::Renderer;
pub use size_buffer::*;
pub use transactions::*;

pub use cosmic_text as text;

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
