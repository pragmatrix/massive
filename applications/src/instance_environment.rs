use derive_more::Constructor;
use massive_renderer::{FontManager, RenderPacing};
use serde_json::{Map, Value};
use tokio::sync::mpsc::UnboundedSender;

use crate::{InstanceCommand, InstanceId};

#[derive(Debug, Clone)]
pub struct InstanceEnvironment {
    pub(crate) command_sender: UnboundedSender<(InstanceId, InstanceSubmission)>,
    // Robustness: This might change on runtime.
    pub(crate) primary_monitor_scale_factor: f64,
    pub(crate) font_manager: FontManager,
    pub(crate) parameters: Map<String, Value>,
}

pub type InstanceParameters = Map<String, Value>;

impl InstanceEnvironment {
    pub fn new(
        requests_tx: UnboundedSender<(InstanceId, InstanceSubmission)>,
        primary_monitor_scale_factor: f64,
        font_manager: FontManager,
    ) -> Self {
        Self {
            command_sender: requests_tx,
            primary_monitor_scale_factor,
            font_manager,
            parameters: Default::default(),
        }
    }

    pub fn with_parameters(mut self, parameters: InstanceParameters) -> Self {
        self.parameters = parameters;
        self
    }
}

#[derive(Debug, Constructor)]
struct InstanceSubmission {
    commands: Vec<InstanceCommand>,
    pacing: RenderPacing,
}

impl InstanceSubmission {
    pub fn into_inner(self) -> (Vec<InstanceCommand>, RenderPacing) {
        (self.commands, self.pacing)
    }
}
