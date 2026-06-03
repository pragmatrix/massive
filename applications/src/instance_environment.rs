use derive_more::Constructor;
use serde_json::{Map, Value};
use tokio::sync::mpsc::UnboundedSender;

use massive_renderer::{FontManager, RenderPacing};
use massive_scene::SceneChange;
use massive_util::ChangeSet;

use crate::{InstanceId, ViewChange, ViewCreationInfo, ViewId};

#[derive(Debug, Clone)]
pub struct InstanceEnvironment {
    pub(crate) submission_sender: UnboundedSender<(InstanceId, InstanceSubmission)>,
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
            submission_sender: requests_tx,
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
pub struct InstanceSubmission {
    changes: ChangeSet<InstanceChange>,
    pacing: RenderPacing,
}

impl InstanceSubmission {
    pub fn into_parts(self) -> (ChangeSet<InstanceChange>, RenderPacing) {
        (self.changes, self.pacing)
    }
}

#[derive(Debug)]
pub enum InstanceChange {
    Scene(SceneChange),
    CreateView(ViewCreationInfo),
    View(ViewId, ViewChange),
    DestroyView(ViewId),
}
