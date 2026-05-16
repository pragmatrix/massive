use derive_more::Debug;

use massive_applications::{InstanceId, InstanceParameters, ViewCreationInfo};

use super::navigation::Direction;
use crate::instance_manager::ViewPath;
use crate::projects::{
    LaunchProfile, LaunchProfileId, MatrixPlacement, ProjectId, ProjectProperties,
};

/// The commands the desktop system can execute.
#[derive(Debug)]
pub enum DesktopCommand {
    Project(ProjectCommand),
    StartInstance {
        launcher: LaunchProfileId,
        parameters: InstanceParameters,
    },
    StopInstance(InstanceId),
    PresentInstance {
        launcher: LaunchProfileId,
        instance: InstanceId,
    },
    PresentView(InstanceId, ViewCreationInfo),
    HideView(ViewPath),
    ZoomOut,
    Navigate(Direction),
}

#[derive(Debug)]
pub enum ProjectCommand {
    AddProject {
        id: ProjectId,
        properties: ProjectProperties,
    },
    #[allow(unused)]
    RemoveProject(ProjectId),
    AddLauncher {
        project: ProjectId,
        id: LaunchProfileId,
        profile: LaunchProfile,
        placement: MatrixPlacement,
    },
    #[allow(unused)]
    RemoveLauncher(LaunchProfileId),
    SetStartupProfile(Option<LaunchProfileId>),
}
