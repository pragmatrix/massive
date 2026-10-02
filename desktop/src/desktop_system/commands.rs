use derive_more::Debug;

use massive_applications::{InstanceId, InstanceParameters, SlotShift};

use super::Direction;
use super::change::Zoom;
use crate::instance_presenter::InstanceRoot;
use crate::projects::{LaunchProfileId, MatrixPlacement, ProjectId, SlotAssignment};

/// The commands the desktop system can execute.
#[derive(Debug)]
pub enum DesktopCommand {
    Project(ProjectCommand),
    /// Present an instance under `launcher`, spawning it if necessary.
    ///
    /// When `root` is `None`, a fresh root is created and the instance is spawned. When `root` is
    /// `Some`, the caller has already spawned the instance, so only presentation happens.
    StartInstance {
        launcher: LaunchProfileId,
        instance: InstanceId,
        root: Option<InstanceRoot>,
        parameters: InstanceParameters,
    },
    StopInstance(InstanceId),

    Navigate(Direction),

    Zoom(Zoom),
}

#[derive(Debug)]
pub enum ProjectCommand {
    /// Assigns slot content, displacing the assigned content according to `shift`.
    ///
    /// A `None` parent creates the assigned project instead — there is no separate
    /// "add project" operation. The root is assigned to the `Desktop` target, which
    /// hosts no slots, so `placement` and `shift` are ignored; only
    /// [`ProjectId::ROOT`] may be assigned there, exactly once.
    AssignSlot {
        parent: Option<ProjectId>,
        placement: MatrixPlacement,
        content: SlotAssignment,
        shift: SlotShift,
    },
    ClearSlot {
        parent: ProjectId,
        placement: MatrixPlacement,
        shift: SlotShift,
    },
    MoveSlot {
        source: (ProjectId, MatrixPlacement),
        dest: (ProjectId, MatrixPlacement),
    },
    SetStartupPath(Option<String>),
}
