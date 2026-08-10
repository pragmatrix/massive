use massive_applications::{InstanceId, InstanceParameters, InstanceSubmission};
use massive_geometry::{SizePx, Vector3};
use massive_util::CollectingVec;

use super::KeyboardFocusReason;
use crate::desktop_system::FocusDepth;
use crate::event_router::EventTransitions;
use crate::instance_presenter::InstanceRoot;
use crate::projects::{
    LaunchProfile, LaunchProfileId, MatrixPlacement, ProjectId, ProjectProperties,
};
use crate::{DesktopTarget, RemoveSlotShiftingPolicy};

pub type Changes = CollectingVec<DesktopChange>;

#[derive(Debug)]
pub enum DesktopChange {
    Project(ProjectChange),
    // Design: SpawnInstance seems to be completely able to run externally outside of the
    // `DesktopSystem`. May introduce something like outside effects that run when transact returns?
    SpawnInstance {
        instance: InstanceId,
        root: InstanceRoot,
        parameters: InstanceParameters,
    },
    // Design: This also could should be done as an external effect.
    ShutdownInstance(InstanceId),
    PresentInstance {
        launcher: LaunchProfileId,
        initial_center_translation: Option<Vector3>,
        instance: InstanceId,
        root: InstanceRoot,
        parameters: InstanceParameters,
    },
    HideInstance {
        launcher: LaunchProfileId,
        instance: InstanceId,
    },
    SetFocus {
        // None: Completely removes the focus from the application.
        target: Option<DesktopTarget>,
        reason: KeyboardFocusReason,
    },
    /// Commits the navigation column affinity. `None` clears it (used by non-navigation focus
    /// changes via `set_focus_change`).
    CommitNavigationAffinity(Option<u32>),
    /// Commit the focus depth.
    CommitFocusDepth(FocusDepth),
    FullScreen(FullScreenChange),
    ResizeAll(SizePx),
    Topology(TopologyChange),
    ForwardEvents(EventTransitions<DesktopTarget>),
    IntegrateInstanceSubmission(InstanceId, InstanceSubmission),
}

#[derive(Debug)]
pub enum Zoom {
    In,
    Out,
    /// Focus on the currently keyboard focused object.
    DefaultForFocused,
}

#[derive(Debug)]
pub enum FullScreenChange {
    NativeStateChanged,
    Enter(InstanceId),
    Exit(InstanceId),
}

impl From<FullScreenChange> for DesktopChange {
    fn from(value: FullScreenChange) -> Self {
        Self::FullScreen(value)
    }
}

#[derive(Debug)]
pub enum ProjectChange {
    AddProject {
        id: ProjectId,
        properties: ProjectProperties,
    },
    RemoveProject(ProjectId),
    AddLauncher {
        project: ProjectId,
        id: LaunchProfileId,
        profile: LaunchProfile,
        placement: MatrixPlacement,
    },
    MoveLauncher {
        launcher: LaunchProfileId,
        placement: MatrixPlacement,
    },
    RemoveLauncher(LaunchProfileId),
    RemoveSlot {
        project: ProjectId,
        placement: MatrixPlacement,
        shifting_policy: RemoveSlotShiftingPolicy,
    },
    SetStartupProfile(Option<LaunchProfileId>),
}

/// Constructs the change(s) for a focus transition.
///
/// Emits `SetFocus`, and — when the focus reason resets navigation affinity — a sibling
/// `SetNavigationAffinity(None)` so the reset flows through change application rather than being
/// applied inline in `focus()`.
pub fn set_focus(target: Option<DesktopTarget>, reason: KeyboardFocusReason) -> Changes {
    let mut changes: Changes = DesktopChange::SetFocus { target, reason }.into();
    if reason.resets_navigation_affinity() {
        changes <<= DesktopChange::CommitNavigationAffinity(None);
    }
    changes
}

#[derive(Debug)]
pub enum TopologyChange {
    // May combine this with Insert?
    Add {
        what: DesktopTarget,
        under: DesktopTarget,
        after: Option<DesktopTarget>,
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
