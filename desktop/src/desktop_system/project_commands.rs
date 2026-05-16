use anyhow::Result;

use massive_shell::Scene;

use super::effects::{DesktopEffect, Effects};
use super::{DesktopSystem, DesktopTarget, ProjectCommand};
use crate::projects::{
    LauncherPresenter, ProjectHeaderPresenter, ProjectMatrixPresenter, ProjectPresenter,
};

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
                let project_target = DesktopTarget::Project(id);

                self.aggregates
                    .hierarchy
                    .add(parent_target.clone(), project_target.clone())?;
                self.aggregates
                    .hierarchy
                    .add_nested(
                        project_target,
                        [DesktopTarget::ProjectHeader(id), DesktopTarget::ProjectMatrix(id)],
                    )?;

                self.aggregates
                    .projects
                    .insert(id, ProjectPresenter::new(parent_location, scene))?;
                let project_location = self
                    .aggregates
                    .projects
                    .get(&id)
                    .expect("Project missing")
                    .location();

                self.aggregates.project_headers.insert(
                    id,
                    ProjectHeaderPresenter::new(
                        properties,
                        project_location.clone(),
                        scene,
                        &mut self.fonts.lock(),
                    ),
                )?;
                self.aggregates
                    .project_matrices
                    .insert(id, ProjectMatrixPresenter::new(project_location, scene))?;
                DesktopEffect::Measure(parent_target).into()
            }
            ProjectCommand::RemoveProject(project) => {
                let effects = self.remove_target(&DesktopTarget::Project(project))?;
                self.aggregates.projects.remove(&project)?;
                self.aggregates.project_headers.remove(&project)?;
                self.aggregates.project_matrices.remove(&project)?;
                effects
            }
            ProjectCommand::AddLauncher {
                project,
                id,
                profile,
                placement,
            } => {
                let matrix_location = self
                    .aggregates
                    .project_matrices
                    .get(&project)
                    .expect("Project missing")
                    .location();
                let presenter = LauncherPresenter::new(
                    matrix_location,
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
                    .add(DesktopTarget::ProjectMatrix(project), id.into())?;
                DesktopEffect::Measure(DesktopTarget::ProjectMatrix(project)).into()
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
