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
    /// Creates a project. The root is created by `AddProject { under: None }`,
    /// which places the project under the `Desktop` target; this is also how an
    /// `AssignSlot` that names a new project materializes it.
    AddProject {
        id: ProjectId,
        name: String,
        /// The placement in the host project's matrix. Ignored for
        /// `under: None` — the root project is not hosted by a matrix.
        placement: MatrixPlacement,
        /// The project whose slot will host the new project; `None` for the root
        /// project under the `Desktop` target.
        under: Option<ProjectId>,
    },
    RemoveProject(ProjectId),
    /// Assigns slot content, displacing the assigned content according to `shift`.
    AssignSlot {
        parent: ProjectId,
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
