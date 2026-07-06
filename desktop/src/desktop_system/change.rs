use massive_applications::{InstanceId, InstanceParameters, InstanceSubmission};
use massive_geometry::Vector3;
use massive_util::CollectingVec;

use crate::{
    DesktopTarget,
    desktop_system::{DesktopCommand, FocusReason, ProjectCommand},
    event_router::EventTransitions,
    instance_presenter::InstanceRoot,
    projects::LaunchProfileId,
};

pub type Changes = CollectingVec<DesktopChange>;
pub type ProjectChange = ProjectCommand;

#[derive(Debug)]
pub enum DesktopChange {
    Project(ProjectChange),
    SpawnInstance {
        instance: InstanceId,
        root: InstanceRoot,
        parameters: InstanceParameters,
    },
    PresentInstance {
        launcher: LaunchProfileId,
        initial_center_translation: Option<Vector3>,
        instance: InstanceId,
        root: InstanceRoot,
    },
    ShutdownInstance(InstanceId),
    HideInstance {
        launcher: LaunchProfileId,
        instance: InstanceId,
    },
    SetFocus {
        // None: Completely removes the focus from the application.
        target: Option<DesktopTarget>,
        reason: FocusReason,
    },
    Topology(TopologyChange),
    ForwardEvents(EventTransitions<DesktopTarget>),
    IntegrateInstanceSubmission(InstanceId, InstanceSubmission),
}

#[derive(Debug)]
pub enum TopologyChange {
    // May combine this with Insert?
    Add {
        what: DesktopTarget,
        under: DesktopTarget,
    },
    AddNested {
        what: Vec<DesktopTarget>,
        under: DesktopTarget,
    },
    Insert {
        what: DesktopTarget,
        at_index: usize,
        under: DesktopTarget,
    },
    /// Sets the focus to the parent if a nested or itself has the focus first. Also removes the
    /// pointer focus.
    Remove(DesktopTarget),
}

impl From<TopologyChange> for DesktopChange {
    fn from(value: TopologyChange) -> Self {
        Self::Topology(value)
    }
}

impl From<ProjectChange> for DesktopChange {
    fn from(value: ProjectChange) -> Self {
        Self::Project(value)
    }
}
