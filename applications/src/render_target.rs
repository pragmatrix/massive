use anyhow::Result;

use massive_animation::AnimationCoordinator;
use massive_scene::SceneChange;

pub trait RenderTarget {
    type Event;

    fn render(
        &mut self,
        changes: Vec<SceneChange>,
        animation_coordinator: &AnimationCoordinator,
        event: Option<Self::Event>,
    ) -> Result<()>;
}
