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
                let parent = parent.map(|p| p.into()).unwrap_or(DesktopTarget::Desktop);
                self.aggregates.hierarchy.add(parent.clone(), id.into())?;
                self.aggregates
                    .groups
                    .insert(id, GroupPresenter::new(properties))?;
                DesktopEffect::RecomputeLayout(parent).into()
            }
            ProjectCommand::RemoveLaunchGroup(group) => {
                let effects = self.remove_target(&group.into())?;
                self.aggregates.groups.remove(&group)?;
                effects
            }
            ProjectCommand::AddLauncher { group, id, profile } => {
                let presenter = LauncherPresenter::new(
                    self.aggregates.project_presenter.location.clone(),
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
