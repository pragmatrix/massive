use massive_applications::InstanceId;
use massive_layout::LayoutTopology;

use crate::projects::{LaunchProfileId, ProjectId};
use crate::{DesktopTarget, OrderedHierarchy};

pub type DesktopTopology = OrderedHierarchy<DesktopTarget>;

impl OrderedHierarchy<DesktopTarget> {
    pub fn launcher_of_instance(&self, instance_id: InstanceId) -> Option<LaunchProfileId> {
        match self.parent(&DesktopTarget::Instance(instance_id)) {
            Some(DesktopTarget::Launcher(id)) => Some(*id),
            _ => None,
        }
    }
    pub fn launcher_of_target(&self, target: &DesktopTarget) -> Option<LaunchProfileId> {
        match target {
            DesktopTarget::Launcher(launcher_id) => Some(*launcher_id),
            DesktopTarget::Instance(instance_id) => self.launcher_of_instance(*instance_id),
            DesktopTarget::View(view_id) => {
                let parent = self.parent_of(&DesktopTarget::View(*view_id))?;
                self.launcher_of_target(parent)
            }
            _ => None,
        }
    }

    pub fn project_of_launcher(&self, launcher_id: LaunchProfileId) -> Option<ProjectId> {
        let target = DesktopTarget::Launcher(launcher_id);
        match self.parent_of(&target) {
            Some(DesktopTarget::ProjectMatrix(project_id)) => Some(*project_id),
            _ => None,
        }
    }

    pub fn project_of_target(&self, target: &DesktopTarget) -> Option<crate::projects::ProjectId> {
        match target {
            DesktopTarget::Project(project_id)
            | DesktopTarget::ProjectHeader(project_id)
            | DesktopTarget::ProjectMatrix(project_id) => Some(*project_id),
            DesktopTarget::Launcher(launcher_id) => self.project_of_launcher(*launcher_id),
            DesktopTarget::Instance(instance_id) => self
                .launcher_of_instance(*instance_id)
                .and_then(|launcher_id| self.project_of_launcher(launcher_id)),
            DesktopTarget::View(view_id) => {
                let parent = self.parent_of(&DesktopTarget::View(*view_id))?;
                self.project_of_target(parent)
            }
            DesktopTarget::Desktop => None,
        }
    }
}
