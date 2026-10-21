use derive_more::Debug;

use massive_applications::{InstanceId, InstanceParameters, SlotShift};

use super::Direction;
use super::change::Zoom;
use crate::instance_presenter::{InstanceKind, InstanceRoot};
use crate::projects::{LaunchProfileId, ProjectId, SlotAssignment, SlotContent, SlotPosition};

/// The commands the desktop system can execute.
#[derive(Debug)]
pub enum DesktopCommand {
    Configuration(ConfigurationCommand),
    /// Present an instance under `launcher`, spawning it if necessary.
    ///
    /// When `root` is `None`, a fresh root is created and the instance is spawned. When `root` is
    /// `Some`, the caller has already spawned the instance, so only presentation happens.
    StartInstance {
        launcher: LaunchProfileId,
        instance: InstanceId,
        root: Option<InstanceRoot>,
        parameters: InstanceParameters,
        /// The instance kind (ADR 0014): an `Assistant` (`Shift+Cmd+T` or `Shift+Cmd+Enter`)
        /// carries its own temporary Full Screen Mode.
        kind: InstanceKind,
    },
    StopInstance(InstanceId),
    /// Adds a default launcher right of the focused slot and focuses it (Cmd+N).
    AddLauncher,
    /// Removes a launcher or a nested project, if no instance runs anywhere below it
    /// (Cmd+Shift+W). Otherwise the content is kept, so that a key press never quits running
    /// processes.
    RemoveSlot(SlotContent),
    /// Wraps the selected slots, or else the focused slot, into a new project (Cmd+G, ADR 0026).
    /// The new project takes the slots' place and receives the focus.
    GroupSlot,
    /// Moves the slots of the focused nested project into its parent's matrix, removes the
    /// project, and selects the ungrouped slots (Cmd+Shift+G, ADR 0026).
    UngroupSlot,

    Navigate(Direction),
    /// Moves the far corner of the slot selection one placement in the direction (Shift+Cmd+arrow,
    /// ADR 0025).
    ExtendSelection(Direction),

    Zoom(Zoom),
    /// Routes a fullscreen request to the focused instance or the native window.
    ToggleFullScreen,
}

#[derive(Debug)]
pub enum ConfigurationCommand {
    /// Assigns slot content, displacing the assigned content according to `shift`.
    ///
    /// A `None` parent creates the assigned project instead — there is no separate
    /// "add project" operation. The root is assigned to the `Desktop` target, which
    /// hosts no slots, so `placement` and `shift` are ignored; only
    /// [`ProjectId::ROOT`] may be assigned there, exactly once.
    AssignSlot {
        parent: Option<ProjectId>,
        placement: SlotPosition,
        content: SlotAssignment,
        shift: SlotShift,
    },
    ClearSlot {
        parent: ProjectId,
        placement: SlotPosition,
        shift: SlotShift,
    },
    MoveSlot {
        source: (ProjectId, SlotPosition),
        dest: (ProjectId, SlotPosition),
    },
    SetStartupPath(Option<String>),
}
