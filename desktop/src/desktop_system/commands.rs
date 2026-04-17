use derive_more::Debug;

use massive_applications::{InstanceId, InstanceParameters, ViewCreationInfo};

use crate::instance_manager::ViewPath;
use crate::navigation;
use crate::projects::{LaunchGroupProperties, LaunchProfile, LaunchProfileId};

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
    Navigate(navigation::Direction),
}

#[derive(Debug)]
pub enum ProjectCommand {
    // Project Configuration
    AddLaunchGroup {
        parent: Option<crate::projects::GroupId>,
        id: crate::projects::GroupId,
        properties: LaunchGroupProperties,
    },
    #[allow(unused)]
    RemoveLaunchGroup(crate::projects::GroupId),
    AddLauncher {
        group: crate::projects::GroupId,
        id: LaunchProfileId,
        profile: LaunchProfile,
    },
    #[allow(unused)]
    RemoveLauncher(LaunchProfileId),
    SetStartupProfile(Option<LaunchProfileId>),
}
