use anyhow::Result;

use massive_animation::AnimationCoordinator;
use massive_scene::SceneChanges;

pub trait RenderTarget {
    fn resize(&mut self, new_size: (u32, u32)) -> Result<()>;

    fn render(
        &mut self,
        changes: SceneChanges,
        animation_coordinator: &AnimationCoordinator,
    ) -> Result<()>;
}
