use std::sync::Arc;

use anyhow::Result;

use massive_geometry::{BoxPx, Color};
use massive_scene::{Location, Ref};

use crate::{
    InstanceChangeCollector, Scene,
    view::{View, ViewRole},
};

#[derive(Debug)]
pub struct ViewBuilder {
    /// The connection to the instance context for submitting changes.
    change_collector: Arc<InstanceChangeCollector>,
    parent: Ref<Location>,
    extent: BoxPx,
    scene: Scene,

    role: ViewRole,

    background_color: Option<Color>,
}

impl ViewBuilder {
    pub(crate) fn new(
        change_collector: Arc<InstanceChangeCollector>,
        parent: Ref<Location>,
        extent: BoxPx,
        scene: Scene,
    ) -> Self {
        Self {
            change_collector,
            parent,
            extent,
            scene,
            role: ViewRole::default(),
            background_color: None,
        }
    }

    pub fn with_role(mut self, role: ViewRole) -> Self {
        self.role = role;
        self
    }

    pub fn with_background_color(mut self, color: Color) -> Self {
        self.background_color = Some(color);
        self
    }

    pub fn build(self) -> Result<View> {
        View::new(
            self.parent,
            self.extent,
            self.scene,
            self.role,
            self.change_collector,
        )
    }
}
