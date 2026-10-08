use massive_applications::{InstanceId, InstanceParameters, InstanceSubmission};
use massive_geometry::{SizePx, Vector3};
use massive_util::CollectingVec;

use super::KeyboardFocusReason;
use crate::DesktopTarget;
use crate::desktop_system::ZoomLevel;
use crate::instance_presenter::{InstanceKind, InstanceRoot};
use crate::projects::{LaunchProfileId, MatrixPlacement, ProjectId, SlotAssignment};
use crate::targeted_event::EventTransitions;
use crate::window_state::WindowState;

/// An effect that the host application executes after the desktop system commits its changes.
#[derive(Debug)]
pub enum DesktopSystemEffect {
    /// Ask the shell to toggle its native window fullscreen state.
    ToggleWindowFullScreen,
    /// Persist the live configuration after a transaction.
    PersistConfiguration,
}

#[derive(Debug)]
pub enum DesktopChange {
    Project(ConfigurationChange),
    // Design: SpawnInstance seems to be completely able to run externally outside of the
    // `DesktopSystem`. May introduce something like outside effects that run when transact returns?
    SpawnInstance {
        instance: InstanceId,
        root: InstanceRoot,
        parameters: InstanceParameters,
    },
    // Design: This could also be done as an external effect.
    ShutdownInstance(InstanceId),
    PresentInstance(InstancePresentation),
    HideInstance {
        launcher: LaunchProfileId,
        instance: InstanceId,
    },
    SetFocus {
        // None: Completely removes the focus from the application.
        target: Option<DesktopTarget>,
    },
    /// Sets the navigation column affinity. `None` clears it (used by non-navigation focus
    /// changes via `set_focus_change`).
    SetNavigationAffinity(Option<u32>),
    /// Sets the camera's zoom level (ADR 0018).
    SetZoomLevel(ZoomLevel),
    /// Toggles the Full Screen Mode of the focused launcher (its primary instances)
    /// or, when an assistant instance is focused, of that instance (ADR 0014).
    ToggleFullScreenMode(ToggleFullScreenModeTarget),
    /// The window state changed; commits it as the system's window state.
    /// The constructor seeds the state, so a resize event commits only
    /// updates.
    WindowResized(WindowState),
    /// Requests the desktop shell to toggle native window fullscreen.
    ToggleWindowFullScreen,
    ResizeAll(SizePx),
    Topology(TopologyChange),
    ForwardEvents(EventTransitions<DesktopTarget>),
    IntegrateInstanceSubmission(InstanceId, InstanceSubmission),
}

pub type Changes = CollectingVec<DesktopChange>;

/// The payload of [`DesktopChange::PresentInstance`]: everything introducing a
/// new instance to the scene carries as one group.
#[derive(Debug)]
pub struct InstancePresentation {
    pub launcher: LaunchProfileId,
    pub initial_center_translation: Option<Vector3>,
    pub instance: InstanceId,
    pub root: InstanceRoot,
    pub parameters: InstanceParameters,
    /// The instance kind (ADR 0014): an assistant carries its own temporary
    /// Full Screen Mode, a primary instance follows its launcher's.
    pub kind: InstanceKind,
}

#[derive(Debug)]
pub enum Zoom {
    In,
    Out,
    /// Points the camera at the focused target at `Focus`; a project target is entered down to its focused
    /// leaf first (ADR 0018).
    Enter,
}

/// What `ToggleFullScreenMode` resolves to when planned. An assistant instance
/// toggles its own temporary mode; a launcher toggles the mode shared by all of
/// its primary instances.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleFullScreenModeTarget {
    Launcher(LaunchProfileId),
    AssistantInstance(InstanceId),
}

#[derive(Debug, Clone)]
pub enum ConfigurationChange {
    /// Assigns `assignment` to the slot `(parent, placement)`. Replacing content is a
    /// clear plus an assign; displacement is expanded into `MoveSlot`s by the
    /// plan, never applied here.
    ///
    /// A `None` parent creates the assigned project instead of slotting it: the
    /// root project is hosted by the `Desktop` target, not a matrix (see
    /// [`DesktopConfiguration::assign_slot`]).
    AssignSlot {
        parent: Option<ProjectId>,
        placement: MatrixPlacement,
        assignment: SlotAssignment,
    },
    /// Empties the slot `(parent, placement)`.
    ClearSlot {
        parent: ProjectId,
        placement: MatrixPlacement,
    },
    /// Moves the content of the source slot to the destination slot. The content
    /// may move to another project's matrix, but not into its own subtree.
    MoveSlot {
        source: (ProjectId, MatrixPlacement),
        dest: (ProjectId, MatrixPlacement),
    },
    SetStartupPath(Option<String>),
}

/// Constructs the change(s) for a focus transition.
///
/// Emits `SetFocus`, and — when the focus reason resets navigation affinity — a sibling
/// `SetNavigationAffinity(None)` so the reset flows through change application rather than being
/// applied inline in `focus()`.
pub fn set_focus(target: Option<DesktopTarget>, reason: KeyboardFocusReason) -> Changes {
    let mut changes: Changes = DesktopChange::SetFocus { target }.into();
    if reason.resets_navigation_affinity() {
        changes <<= DesktopChange::SetNavigationAffinity(None);
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

impl From<ConfigurationChange> for DesktopChange {
    fn from(value: ConfigurationChange) -> Self {
        Self::Project(value)
    }
}
