use derive_more::Debug;

use massive_applications::{InstanceId, InstanceParameters, McpConfiguration, SlotShift};

use super::Direction;
use super::change::Zoom;
use crate::instance_presenter::{InstanceKind, InstanceRoot};
use crate::projects::{
    LaunchProfileId, MatrixRect, ProjectId, SlotAssignment, SlotContent, SlotPosition,
};

/// The commands the desktop system can execute.
#[derive(Debug)]
pub enum DesktopCommand {
    Configuration(ConfigurationCommand),
    /// Applies a configuration operation to the selection, which always contains the
    /// keyboard-focused slot. A key cannot know which slots the selection covers, so the desktop
    /// system builds the [`ConfigurationCommand`] the operation stands for (ADR 0024).
    ConfigureSelection(SelectionOperation),
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
        /// With [`FocusIntent::Follow`], the instance takes the focus and the camera zooms to it.
        focus: FocusIntent,
    },
    StopInstance(InstanceId),

    Navigate(Direction),
    /// Extends the slot selection by one slot in the direction (Shift+Cmd+arrow, ADR 0025).
    ExtendSelection(Direction),
    /// Selects all assigned slots of the project, around the focused slot (Cmd+A, ADR 0030).
    SelectAll(ProjectId),
    /// Collapses the selection to the focused slot (Esc, ADR 0030).
    CollapseSelection,
    /// Escapes the focused instance, launcher, or project by one level (Esc, Cmd+Esc, ADR 0031).
    Escape,

    Zoom(Zoom),
    /// Routes a fullscreen request to the focused instance or the native window.
    ToggleFullScreen,
}

/// A configuration operation a key applies to the selection, before the desktop system knows which
/// slots the selection covers (ADR 0024). Grouping, ungrouping, and moving act on the whole
/// selection. Adding and removing act on its keyboard-focused slot only. Each operation stands for
/// one [`ConfigurationCommand`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionOperation {
    /// Adds a default launcher right of the focused slot (Cmd+N).
    AddLauncher,
    /// Removes the focused launcher or nested project, if no instance runs anywhere below it
    /// (Cmd+Shift+W). Otherwise nothing is removed, so that a key press never quits running
    /// processes.
    Remove,
    /// Wraps the selected slots, or else the focused slot, into a new project (Cmd+G, ADR 0026).
    Group,
    /// Ungroups the focused nested project, or the project whose slots are all selected
    /// (Cmd+Shift+G, ADR 0026, ADR 0029).
    Ungroup,
    /// Moves the selected slots one slot in the direction, and the selection with them
    /// (Ctrl+Shift+Cmd+arrow, ADR 0033).
    Move(Direction),
}

/// Whether a command that creates or moves content takes the keyboard focus there (ADR 0036). The
/// keyboard and the CLI follow, so the user lands on what they made. MCP keeps the focus and the
/// camera where they are, so an agent never pulls the user away. A focused target that a command
/// removes hands its focus on either way. The selection survives either way, unless the command
/// changes the slots of the project that holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusIntent {
    Follow,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the MCP tools that issue it come with the next step"
        )
    )]
    Keep,
}

/// Every operation a user can perform on the configuration (ADR 0024). A command names its slots
/// by ids and slot positions only. The keyboard, the CLI, and the boot replay turn what they
/// mean into these, and the desktop system plans each command in one place, together with where the
/// focus goes afterwards.
#[derive(Debug)]
pub enum ConfigurationCommand {
    /// Assigns slot content, displacing the assigned content according to `shift`.
    ///
    /// A `None` parent creates the assigned project instead — there is no separate
    /// "add project" operation. The root is assigned to the `Desktop` target, which
    /// hosts no slots, so `slot` and `shift` are ignored; only
    /// [`ProjectId::ROOT`] may be assigned there, exactly once.
    AssignSlot {
        parent: Option<ProjectId>,
        slot: SlotPosition,
        content: SlotAssignment,
        shift: SlotShift,
    },
    /// Unassigns a slot. The instances that run below its content are stopped. When the focus was
    /// inside the content, it moves to the nearest assigned slot, or else to the hosting project
    /// (ADR 0029).
    ClearSlot {
        parent: ProjectId,
        slot: SlotPosition,
        shift: SlotShift,
    },
    /// Moves a slot's content to an unassigned slot, possibly in another project. Moving onto an
    /// assigned slot is a [`ConfigurationCommand::SwapSlots`] instead, so a move never displaces
    /// content.
    MoveSlot {
        source: (ProjectId, SlotPosition),
        dest: (ProjectId, SlotPosition),
    },
    /// Exchanges the contents of two assigned slots of one project.
    SwapSlots {
        parent: ProjectId,
        a: SlotPosition,
        b: SlotPosition,
    },
    /// Moves the `block`, a rectangle of `parent`'s matrix, one slot in `direction`. The contents in
    /// front of the block take the place it leaves, and at the matrix's edges nothing moves, so a
    /// move never grows the matrix (ADR 0033).
    MoveSlots {
        parent: ProjectId,
        block: MatrixRect,
        direction: Direction,
    },
    /// Pushes a slot's content one slot in `direction`, together with the contents in its way.
    PushSlot {
        parent: ProjectId,
        slot: SlotPosition,
        direction: Direction,
    },
    /// Assigns `content` right of the slot that `anchor` sits in, shifting the row. With
    /// [`FocusIntent::Follow`], it focuses the content, so that new content appears where the user
    /// works.
    AddNextTo {
        anchor: SlotContent,
        content: SlotAssignment,
        focus: FocusIntent,
    },
    /// Wraps `parent`'s `slots` into a new project named `name`, which takes their place (ADR 0026).
    /// With [`FocusIntent::Follow`], the project receives the focus.
    Group {
        parent: ProjectId,
        slots: Vec<SlotPosition>,
        name: String,
        focus: FocusIntent,
    },
    /// Moves the slots of a nested project into its parent's matrix and removes the project
    /// (ADR 0026). With [`FocusIntent::Follow`], it selects the ungrouped slots. An empty project is
    /// removed instead (ADR 0029).
    Ungroup {
        project: ProjectId,
        focus: FocusIntent,
    },
    SetStartupPath(Option<String>),
    /// Makes `project` an MCP root configured by `mcp`, or no MCP root when it is `None`
    /// (ADR 0036). Enabling, disabling, and changing the access scopes are all this command. It is
    /// refused when it would make the root project an MCP root or nest MCP roots.
    ConfigureMcp {
        project: ProjectId,
        mcp: Option<McpConfiguration>,
    },
}
