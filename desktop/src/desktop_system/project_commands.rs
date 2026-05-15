use anyhow::Result;

use massive_shell::Scene;

use super::effects::{DesktopEffect, Effects};
use super::{DesktopSystem, DesktopTarget, ProjectCommand};
use crate::projects::{GroupPresenter, LauncherPresenter};

impl DesktopSystem {
    pub(super) fn apply_project_command(
        &mut self,
        command: ProjectCommand,
        scene: &Scene,
    ) -> Result<Effects> {
        let effects = match command {
            ProjectCommand::AddLaunchGroup {
                parent,
                id,
                properties,
            } => {
                let (parent_target, parent_location) = match parent {
                    Some(parent_group) => {
                        let parent_location = self
                            .aggregates
                            .groups
                            .get(&parent_group)
                            .expect("Parent group missing")
                            .location();
                        (DesktopTarget::Group(parent_group), parent_location)
                    }
                    None => (
                        DesktopTarget::Desktop,
                        self.aggregates.project_presenter.location.clone(),
                    ),
                };

                self.aggregates.hierarchy.add(parent_target.clone(), id.into())?;
                self.aggregates.groups.insert(
                    id,
                    GroupPresenter::new(properties, parent_location, scene),
                )?;
                DesktopEffect::RecomputeLayout(parent_target).into()
            }
            ProjectCommand::RemoveLaunchGroup(group) => {
                let effects = self.remove_target(&group.into())?;
                self.aggregates.groups.remove(&group)?;
                effects
            }
            ProjectCommand::AddLauncher { group, id, profile } => {
                let group_location = self
                    .aggregates
                    .groups
                    .get(&group)
                    .expect("Group missing")
                    .location();
                let presenter = LauncherPresenter::new(
                    group_location,
                    id,
                    profile,
                    massive_geometry::Size::default(),
                    scene,
                    &mut self.fonts.lock(),
                );
                self.aggregates.launchers.insert(id, presenter)?;

                self.aggregates.hierarchy.add(group.into(), id.into())?;
                DesktopEffect::RecomputeLayout(DesktopTarget::Group(group)).into()
            }
            ProjectCommand::RemoveLauncher(id) => {
                let target = DesktopTarget::Launcher(id);
                let effects = self.remove_target(&target)?;

                self.aggregates.launchers.remove(&id)?;
                effects
            }
            ProjectCommand::SetStartupProfile(launch_profile_id) => {
                self.aggregates.startup_profile = launch_profile_id;
                Effects::None
            }
        };

        Ok(effects)
    }
}
