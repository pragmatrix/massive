use massive_applications::InstanceId;
use massive_layout::LayoutTopology;

use crate::projects::{LaunchProfileId, ProjectId};
use crate::{DesktopTarget, OrderedHierarchy};

pub type DesktopTopology = OrderedHierarchy<DesktopTarget>;

impl OrderedHierarchy<DesktopTarget> {
    pub fn launcher_of_instance(&self, instance_id: InstanceId) -> LaunchProfileId {
        match self.parent(&DesktopTarget::Instance(instance_id)) {
            Some(DesktopTarget::Launcher(id)) => *id,
            parent => {
                panic!("instance {instance_id:?} must be nested under a launcher, found {parent:?}")
            }
        }
    }

    pub fn instance_of_target(&self, target: &DesktopTarget) -> Option<InstanceId> {
        match target {
            DesktopTarget::Instance(instance_id) => Some(*instance_id),
            _ => self
                .parent_of(target)
                .and_then(|parent| self.instance_of_target(parent)),
        }
    }

    pub fn launcher_of_target(&self, target: &DesktopTarget) -> Option<LaunchProfileId> {
        match target {
            DesktopTarget::Launcher(launcher_id) => Some(*launcher_id),
            DesktopTarget::Instance(instance_id) => Some(self.launcher_of_instance(*instance_id)),
            DesktopTarget::View(view_id) => {
                let parent = self.parent_of(&DesktopTarget::View(*view_id))?;
                self.launcher_of_target(parent)
            }
            _ => None,
        }
    }

    pub fn project_of_launcher(&self, launcher_id: LaunchProfileId) -> ProjectId {
        let target = DesktopTarget::Launcher(launcher_id);
        match self.parent_of(&target) {
            Some(DesktopTarget::ProjectMatrix(project_id)) => *project_id,
            parent => {
                panic!(
                    "launcher {launcher_id:?} must be nested under a project matrix, found {parent:?}"
                )
            }
        }
    }

    pub fn project_of_target(&self, target: &DesktopTarget) -> Option<ProjectId> {
        match target {
            DesktopTarget::Project(project_id)
            | DesktopTarget::ProjectHeader(project_id)
            | DesktopTarget::ProjectMatrix(project_id) => Some(*project_id),
            DesktopTarget::Launcher(launcher_id) => Some(self.project_of_launcher(*launcher_id)),
            DesktopTarget::Instance(instance_id) => {
                Some(self.project_of_launcher(self.launcher_of_instance(*instance_id)))
            }
            DesktopTarget::View(view_id) => {
                let parent = self.parent_of(&DesktopTarget::View(*view_id))?;
                self.project_of_target(parent)
            }
            DesktopTarget::Desktop => None,
        }
    }

    pub fn matrix_launchers(&self, project_id: ProjectId) -> impl Iterator<Item = LaunchProfileId> {
        self.get_nested(&DesktopTarget::ProjectMatrix(project_id))
            .iter()
            .map(|target| match target {
                DesktopTarget::Launcher(launcher_id) => *launcher_id,
                _ => panic!("project matrix children must be launchers"),
            })
    }

    pub fn launcher_instances(&self, launcher_id: LaunchProfileId) -> Vec<InstanceId> {
        self.get_nested(&DesktopTarget::Launcher(launcher_id))
            .iter()
            .map(|target| match target {
                DesktopTarget::Instance(instance_id) => *instance_id,
                _ => panic!("launcher children must be instances"),
            })
            .collect()
    }
}
