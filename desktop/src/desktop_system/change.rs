use massive_applications::{InstanceId, InstanceParameters, InstanceSubmission, McpConfiguration};
use massive_geometry::{SizePx, Vector3};
use massive_util::{CollectingSet, CollectingVec};

use super::selection::Selection;
use super::{FocusableTarget, InputFocus, KeyboardFocusReason};
use crate::DesktopTarget;
use crate::desktop_system::ZoomLevel;
use crate::instance_presenter::{InstanceKind, InstanceRoot};
use crate::projects::{LaunchProfileId, ProjectId, SlotAssignment, SlotContent, SlotPosition};
use crate::targeted_event::EventTransitions;
use crate::window_state::WindowState;

/// An effect that the host application executes after the desktop system commits its changes.
#[derive(Debug)]
pub enum DesktopSystemEffect {
    /// Ask the shell to toggle its native window fullscreen state.
    ToggleWindowFullScreen,
    /// Persist the live configuration after a transaction.
    PersistConfiguration,
    /// MCP roots were configured, reconfigured, disabled, or removed (ADR 0036). The tool host
    /// closes the tool connections bound to them.
    McpRootsChanged(CollectingSet<ProjectId>),
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
        target: Option<FocusableTarget>,
    },
    /// Sets the navigation column affinity. `None` clears it (used by non-navigation focus
    /// changes via `set_focus_change`).
    SetNavigationAffinity(Option<u32>),
    /// Sets the slot selection around the focused slot (ADR 0025, ADR 0030).
    SetSelection(Selection),
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
    /// Remembers where the last typed key went (ADR 0020).
    SetInputFocus(InputFocus),
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
    /// Assigns `assignment` to `parent`'s `slot`. Replacing content is a
    /// clear plus an assign; displacement is expanded into `MoveSlot`s by the
    /// plan, never applied here.
    ///
    /// A `None` parent creates the assigned project instead of slotting it: the
    /// root project is hosted by the `Desktop` target, not a matrix (see
    /// [`DesktopConfiguration::assign_slot`]).
    AssignSlot {
        parent: Option<ProjectId>,
        slot: SlotPosition,
        assignment: SlotAssignment,
    },
    /// Unassigns `parent`'s `slot`.
    ClearSlot {
        parent: ProjectId,
        slot: SlotPosition,
    },
    /// Moves the content of the source slot to the destination slot. The content
    /// may move to another project's matrix, but not into its own subtree.
    MoveSlot {
        source: (ProjectId, SlotPosition),
        dest: (ProjectId, SlotPosition),
    },
    /// Wraps `parent`'s `slots` into a new project, which takes their place (ADR 0026). Grouping
    /// needs its own change, because the project's slot is usually one of the grouped slots. Expressed as assignments and moves, one of them would first have
    /// to wait in a temporary slot.
    Group {
        parent: ProjectId,
        slots: Vec<SlotPosition>,
        project: ProjectId,
        name: String,
    },
    /// Moves the slots of the project in `parent`'s `slot` into `parent` and removes the project,
    /// the inverse of `Group`. It needs its own change for the same reason: the project's first
    /// slot usually lands on the project's own slot. Slots in the way are pushed aside by
    /// `MoveSlot`s planned before it.
    Ungroup {
        parent: ProjectId,
        slot: SlotPosition,
    },
    /// Exchanges the contents of two slots in `parent`'s matrix. A swap needs its own change,
    /// because each content needs the other's slot at the same time. Expressed as moves, one
    /// of them would first have to wait in a temporary slot.
    SwapSlots {
        parent: ProjectId,
        a: SlotPosition,
        b: SlotPosition,
    },
    /// Moves the contents of slots in `parent`'s matrix, each from the first slot of a pair in
    /// `landings` to the second, all at once (ADR 0033). A moved block needs its own change for the
    /// same reason as a swap: its content and the strip that jumps behind it need each other's
    /// slots. Only moving the selection plans it, which is why it keeps the selection (see
    /// [`ConfigurationChange::keeps_selection`]).
    RelocateSlots {
        parent: ProjectId,
        landings: Vec<(SlotPosition, SlotPosition)>,
    },
    SetStartupPath(Option<String>),
    ConfigureMcp {
        project: ProjectId,
        mcp: Option<McpConfiguration>,
    },
}

impl ConfigurationChange {
    /// Whether the selection survives this change, even when the change edits the project that
    /// holds the selection (ADR 0025, ADR 0033). The selection is stored as offsets around the
    /// focused slot. A change that adds, removes, or moves slots of that project could make those
    /// offsets cover other slots, or reach past the matrix's edge when the focused slot itself
    /// moves, so the transaction collapses the selection after it.
    pub fn keeps_selection(&self) -> bool {
        match self {
            Self::AssignSlot { .. }
            | Self::ClearSlot { .. }
            | Self::MoveSlot { .. }
            | Self::Group { .. }
            | Self::Ungroup { .. }
            | Self::SwapSlots { .. } => false,
            // The focused slot moves with the selection's block, so the same offsets cover the
            // moved block.
            Self::RelocateSlots { .. } => true,
            // No matrix changes.
            Self::SetStartupPath(_) | Self::ConfigureMcp { .. } => true,
        }
    }

    /// The projects whose slots this change edits, so the shared column counts are refreshed
    /// for them (ADR 0035). A project the change creates in a slot is not listed, because
    /// refreshing the project hosting that slot takes it in. The projects a change removes are
    /// named where it is applied, because only the removal knows the subtree it takes along.
    pub fn touched_projects(&self) -> CollectingSet<ProjectId> {
        match self {
            // Without a parent, the assignment creates the root, which no slot hosts.
            Self::AssignSlot {
                parent: None,
                assignment,
                ..
            } => match assignment.content() {
                SlotContent::Project(root) => root.into(),
                SlotContent::Launcher(_) => {
                    unreachable!("a launcher is always assigned into a project's slot")
                }
            },
            Self::AssignSlot {
                parent: Some(parent),
                ..
            }
            | Self::ClearSlot { parent, .. }
            | Self::Ungroup { parent, .. }
            | Self::SwapSlots { parent, .. }
            | Self::RelocateSlots { parent, .. } => (*parent).into(),
            Self::MoveSlot {
                source: (source, _),
                dest: (dest, _),
            } => [*source, *dest].into_iter().collect(),
            // The new project is touched too, because it takes slots from its parent.
            Self::Group {
                parent, project, ..
            } => [*parent, *project].into_iter().collect(),
            Self::SetStartupPath(_) | Self::ConfigureMcp { .. } => CollectingSet::default(),
        }
    }

    /// The projects whose subtrees may show different MCP indicators after this change
    /// (ADR 0036). Configuring MCP changes the project's own indicators and those of everything it
    /// covers, and a move into another project can take content into or out of an MCP root. Every
    /// other change keeps its content inside the same MCP root, or outside every MCP root.
    pub fn mcp_indicator_projects(&self) -> CollectingSet<ProjectId> {
        match self {
            Self::ConfigureMcp { project, .. } => (*project).into(),
            Self::MoveSlot {
                source: (source, _),
                dest: (dest, _),
            } if source != dest => (*dest).into(),
            Self::AssignSlot { .. }
            | Self::ClearSlot { .. }
            | Self::MoveSlot { .. }
            | Self::Group { .. }
            | Self::Ungroup { .. }
            | Self::SwapSlots { .. }
            | Self::RelocateSlots { .. }
            | Self::SetStartupPath(_) => CollectingSet::default(),
        }
    }
}

/// Constructs the change(s) for a focus transition.
///
/// Emits `SetFocus`, and — when the focus reason resets navigation affinity — a sibling
/// `SetNavigationAffinity(None)` so the reset flows through change application rather than being
/// applied inline in `focus()`.
pub fn set_focus(target: Option<FocusableTarget>, reason: KeyboardFocusReason) -> Changes {
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
    /// Moves a target and everything below it under another parent. The keyboard focus is left
    /// alone, because all moved targets stay in the topology.
    Reparent {
        what: DesktopTarget,
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
