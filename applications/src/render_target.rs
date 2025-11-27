use anyhow::Result;

use massive_scene::SceneChanges;

pub trait RenderTarget {
    fn render(&mut self, changes: SceneChanges, pacing: RenderPacing) -> Result<()>;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum RenderPacing {
    #[default]
    // Render as fast as possible to be able to represent input changes.
    Fast,
    // Render a smooth as possible so that animations are synced to the frame rate.
    Smooth,
}
