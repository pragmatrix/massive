use massive_applications::{InstanceId, InstanceParameters};
use massive_geometry::Vector3;

use crate::{
    DesktopTarget,
    desktop_system::{DesktopCommand, FocusReason},
    instance_presenter::InstanceRoot,
    projects::LaunchProfileId,
};

#[derive(Debug)]
pub enum DesktopChange {
    // Temporarily
    Cmd(DesktopCommand),
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
}

#[derive(Debug)]
pub enum TopologyChange {
    Insert {
        what: DesktopTarget,
        at_index: usize,
        into: DesktopTarget,
    },
    /// Sets the focus to the parent if a nested or itself has the focus first. Also removes the
    /// pointer focus.
    Remove(DesktopTarget),
}
