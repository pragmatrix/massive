use anyhow::{Result, bail};
use derive_more::Constructor;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::mpsc::UnboundedSender;

use massive_renderer::RenderPacing;
use massive_scene::{Location, Ref, SceneChange};
use massive_util::ChangeSet;

use crate::{InstanceId, ViewChange, ViewCreationInfo, ViewId, ViewRole};

/// Shared instance configuration cloned into each spawned instance.
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

/// Requests sent by the terminal CLI as JSON over its desktop request stream.
#[derive(Debug, Serialize, Deserialize)]
pub enum ConfigurationRequest {
    AddLauncher,
    Assign {
        target: ConfigurationTarget,
        name: String,
        column: u32,
        row: u32,
        under: Option<String>,
        #[serde(default)]
        shift: SlotShift,
    },
    /// `name` selects a specific project or launcher without focusing it first.
    Remove {
        target: ConfigurationTarget,
        name: Option<String>,
    },
    MoveLauncher {
        direction: MoveDirection,
    },
    PushLauncher {
        direction: MoveDirection,
    },
    SetStartup {
        path: Option<String>,
    },
    Resize {
        size_px: (u32, u32),
    },
    Undo,
    Redo,
}

/// The kind of content an assignment or removal request addresses.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ConfigurationTarget {
    Project,
    Launcher,
}

/// Whether assignment or clearing shifts neighboring slot content.
///
/// This is planning policy carried by a request; the dispatcher expands it into
/// concrete slot moves before applying configuration changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SlotShift {
    #[default]
    Shift,
    Keep,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum MoveDirection {
    Left,
    Right,
    Up,
    Down,
}
