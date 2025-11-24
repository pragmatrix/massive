use anyhow::Result;

use massive_animation::AnimationCoordinator;
use massive_scene::SceneChanges;

pub trait RenderTarget {
    type Event;

    fn render(
        &mut self,
        changes: SceneChanges,
        animation_coordinator: &AnimationCoordinator,
        event: Option<Self::Event>,
    ) -> Result<()>;
}
