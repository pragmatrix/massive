use anyhow::Result;

use massive_geometry::{BoxPx, Color};
use massive_scene::{Location, Ref};

use crate::view::{View, ViewRole};

#[derive(Debug)]
pub struct ViewBuilder {
    parent: Ref<Location>,
    extent: BoxPx,

    role: ViewRole,

    background_color: Option<Color>,
}

impl ViewBuilder {
    pub(crate) fn new(parent: Ref<Location>, extent: BoxPx) -> Self {
        Self {
            parent,
            extent,
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
        View::new(self.parent, self.extent, self.role)
    }
}
