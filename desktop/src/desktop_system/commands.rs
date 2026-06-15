use derive_more::Debug;

use massive_applications::{InstanceId, InstanceParameters, InstanceSubmission};

use super::navigation::Direction;
use crate::instance_presenter::InstanceRoot;
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
        root: InstanceRoot,
    },
    IntegrateInstanceSubmission(InstanceId, InstanceSubmission),
    ZoomIn,
    ZoomOut,
    Navigate(Direction),
}

impl DesktopCommand {
    pub fn is_navigation(&self) -> bool {
        match self {
            Self::ZoomIn | Self::ZoomOut | Self::Navigate(_) => true,
            Self::Project(_)
            | Self::StartInstance { .. }
            | Self::StopInstance(_)
            | Self::PresentInstance { .. }
            | Self::IntegrateInstanceSubmission(_, _) => false,
        }
    }

    pub fn is_keyboard_command(&self) -> bool {
        match self {
            Self::StartInstance { .. }
            | Self::StopInstance(_)
            | Self::ZoomIn
            | Self::ZoomOut
            | Self::Navigate(_) => true,
            Self::Project(_)
            | Self::PresentInstance { .. }
            | Self::IntegrateInstanceSubmission(_, _) => false,
        }
    }
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
