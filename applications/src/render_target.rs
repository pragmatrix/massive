use anyhow::Result;

use massive_scene::SceneChange;

pub trait RenderTarget {
    type Event;

    fn render(
        &mut self,
        changes: Vec<SceneChange>,
        animations_active: bool,
        event: Option<Self::Event>,
    ) -> Result<()>;
}
