use anyhow::Result;

use massive_shell::Scene;

use super::effects::{DesktopEffect, Effects};
use super::{DesktopSystem, DesktopTarget, ProjectCommand};
use crate::projects::{LauncherPresenter, ProjectPresenter};

impl DesktopSystem {
    pub(super) fn apply_project_command(
        &mut self,
        command: ProjectCommand,
        scene: &Scene,
    ) -> Result<Effects> {
        let effects = match command {
            ProjectCommand::AddProject { id, properties } => {
                let parent_target = DesktopTarget::Desktop;
                let parent_location = self.aggregates.desktop_presenter.location.clone();

                self.aggregates
                    .hierarchy
                    .add(parent_target.clone(), DesktopTarget::Project(id))?;
                self.aggregates.projects.insert(
                    id,
                    ProjectPresenter::new(
                        properties,
                        parent_location,
                        scene,
                        &mut self.fonts.lock(),
                    ),
                )?;
                DesktopEffect::Measure(parent_target).into()
            }
            ProjectCommand::RemoveProject(project) => {
                let effects = self.remove_target(&DesktopTarget::Project(project))?;
                self.aggregates.projects.remove(&project)?;
                effects
            }
            ProjectCommand::AddLauncher {
                project,
                id,
                profile,
                placement,
            } => {
                let project_location = self
                    .aggregates
                    .projects
                    .get(&project)
                    .expect("Project missing")
                    .location();
                let presenter = LauncherPresenter::new(
                    project_location,
                    id,
                    profile,
                    massive_geometry::Size::default(),
                    scene,
                    &mut self.fonts.lock(),
                );
                self.aggregates.launchers.insert(id, presenter)?;
                self.aggregates.launcher_placements.insert(id, placement)?;

                self.aggregates
                    .hierarchy
                    .add(DesktopTarget::Project(project), id.into())?;
                DesktopEffect::Measure(DesktopTarget::Project(project)).into()
            }
            ProjectCommand::RemoveLauncher(id) => {
                let target = DesktopTarget::Launcher(id);
                let effects = self.remove_target(&target)?;

                self.aggregates.launchers.remove(&id)?;
                self.aggregates.launcher_placements.remove(&id)?;
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
