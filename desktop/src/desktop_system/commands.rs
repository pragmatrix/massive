use derive_more::Debug;

use massive_applications::{InstanceId, InstanceParameters};

use super::Direction;
use crate::instance_presenter::InstanceRoot;
use crate::projects::{
    LaunchProfile, LaunchProfileId, MatrixPlacement, ProjectId, ProjectProperties,
};

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

    ZoomIn,
    ZoomOut,
    ResetZoom,
}

#[derive(Debug)]
pub enum ProjectCommand {
    AddProject {
        id: ProjectId,
        properties: ProjectProperties,
        after: Option<ProjectId>,
    },
    RemoveProject(ProjectId),
    AddLauncher {
        project: ProjectId,
        id: LaunchProfileId,
        profile: LaunchProfile,
        placement: MatrixPlacement,
    },
    RemoveLauncher(LaunchProfileId),
    SetStartupProfile(Option<LaunchProfileId>),
}
