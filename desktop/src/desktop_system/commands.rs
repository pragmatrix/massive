use derive_more::Debug;

use massive_applications::{InstanceId, InstanceParameters, InstanceSubmission};

use super::DesktopTarget;
use super::navigation::Direction;
use crate::event_router::NavigationTarget;
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
    NavigateTo(Option<NavigationTarget<DesktopTarget>>),
    Navigate(Direction),
}

impl DesktopCommand {
    pub fn resets_zoom(&self) -> bool {
        matches!(self, Self::StartInstance { .. } | Self::StopInstance(_))
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
