use anyhow::Result;
use tokio::sync::mpsc::UnboundedSender;

use massive_geometry::Color;

use crate::{
    InstanceId, Scene,
    instance_context::InstanceCommand,
    view::{View, ViewRole},
};

#[derive(Debug)]
pub struct ViewBuilder {
    command_sender: UnboundedSender<(InstanceId, InstanceCommand)>,
    instance: InstanceId,

    role: ViewRole,
    size: (u32, u32),

    background_color: Option<Color>,
}

impl ViewBuilder {
    pub(crate) fn new(
        requests: UnboundedSender<(InstanceId, InstanceCommand)>,
        instance: InstanceId,
        size: (u32, u32),
    ) -> Self {
        Self {
            command_sender: requests,
            instance,
            size,
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

    pub fn build(self, scene: &Scene) -> Result<View> {
        View::new(
            self.instance,
            self.command_sender,
            self.role,
            self.size,
            scene,
        )
    }
}
