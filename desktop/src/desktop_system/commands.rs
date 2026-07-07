use derive_more::Debug;

use massive_applications::{InstanceId, InstanceParameters};

use super::navigation::Direction;
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

    ZoomIn,
    ZoomOut,
    /// A navigation request caused by an input event (like clicking on a target).
    // NavigateTo(Option<NavigationTarget<DesktopTarget>>),
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
