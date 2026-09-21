use anyhow::{Result, bail};
use derive_more::Constructor;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::mpsc::UnboundedSender;

use massive_renderer::RenderPacing;
use massive_scene::{Location, Ref, SceneChange};
use massive_util::ChangeSet;

use crate::{InstanceId, ViewChange, ViewCreationInfo, ViewId, ViewRole};

/// Shared instance configuration cloned into each spawned instance. Fonts are not part of it: the
/// manager is reached through the task's shaping context (ADR 0005), and each instance derives its
/// own scratch from it (ADR 0006).
#[derive(Debug, Clone)]
pub struct InstanceEnvironment {
    pub submission_sender: UnboundedSender<(InstanceId, InstanceSubmission)>,
    // Robustness: This might change on runtime.
    pub primary_monitor_scale_factor: f64,
    pub parameters: Map<String, Value>,
}

pub type InstanceParameters = Map<String, Value>;

impl InstanceEnvironment {
    pub fn new(
        requests_tx: UnboundedSender<(InstanceId, InstanceSubmission)>,
        primary_monitor_scale_factor: f64,
    ) -> Self {
        Self {
            submission_sender: requests_tx,
            primary_monitor_scale_factor,
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
    /// Submission-level render metadata, including for empty change sets.
    pacing: RenderPacing,
}

impl InstanceSubmission {
    pub fn primary_view_creation_info(&self) -> Result<Option<ViewCreationInfo>> {
        let mut primary_view_creation_info = None;

        for change in self.changes.iter() {
            let InstanceChange::CreateView(info) = change else {
                continue;
            };

            if info.role != ViewRole::Primary {
                continue;
            }

            if primary_view_creation_info.replace(info.clone()).is_some() {
                bail!("Submission created multiple primary views");
            }
        }

        Ok(primary_view_creation_info)
    }

    pub fn changes(&self) -> impl Iterator<Item = &InstanceChange> {
        self.changes.iter()
    }

    pub fn into_parts(self) -> (ChangeSet<InstanceChange>, RenderPacing) {
        (self.changes, self.pacing)
    }
}

#[derive(Debug)]
pub enum InstanceChange {
    Scene(SceneChange),

    // Design: Combine the following three?
    CreateView(ViewCreationInfo),
    View(ViewId, ViewChange),
    DestroyView(ViewId),

    /// Design: This should probably converted to a kind of custom boxed command / request
    /// (discriminated by type), so that the interface stays abstract over what outer shell is
    /// driving the instance.
    Configuration(ConfigurationRequest),

    /// The instance ended. The `Ref<Location>` can just be dropped now as soon this event got
    /// received (and so may enqueue its deletion into the `ChangeCollector` after all other events
    /// have been received).
    End(Ref<Location>),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ConfigurationRequest {
    AddProject,
    // `name` is for removing a specific project without selecting it first.
    RemoveProject { name: Option<String> },
    AddLauncher,
    // `name` is for removing a specific launcher without selecting it first.
    RemoveLauncher { name: Option<String> },
    MoveLauncher { direction: MoveDirection },
    PushLauncher { direction: MoveDirection },
    Resize { size_px: (u32, u32) },
    Undo,
    Redo,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum MoveDirection {
    Left,
    Right,
    Up,
    Down,
}
