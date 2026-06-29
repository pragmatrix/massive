use anyhow::Result;

use massive_shell::Scene;

use super::{DesktopSystem, DesktopTarget, ProjectCommand};
use crate::desktop_system::effects::MeasureSet;
use crate::projects::{LauncherPresenter, ProjectPresenter};

impl DesktopSystem {
    pub(super) fn apply_project_command(
        &mut self,
        command: ProjectCommand,
        scene: &Scene,
    ) -> Result<MeasureSet> {
        let measure_set = match command {
            ProjectCommand::AddProject { id, properties } => {
                let parent_target = DesktopTarget::Desktop;
                let parent_location = self.desktop_presenter.location.clone();
                let project_target = DesktopTarget::Project(id);

                self.aggregates
                    .hierarchy
                    .add(parent_target.clone(), project_target.clone())?;
                self.aggregates.hierarchy.add_nested(
                    project_target,
                    [
                        DesktopTarget::ProjectHeader(id),
                        DesktopTarget::ProjectMatrix(id),
                    ],
                )?;

                self.aggregates.projects.insert(
                    id,
                    ProjectPresenter::new(
                        properties,
                        parent_location,
                        scene,
                        &mut self.fonts.lock(),
                    ),
                )?;
                parent_target.into()
            }
            ProjectCommand::RemoveProject(project) => {
                let measures = self.remove_target(&DesktopTarget::Project(project))?;
                self.aggregates.projects.remove(&project)?;
                measures
            }
            ProjectCommand::AddLauncher {
                project,
                id,
                profile,
                placement,
            } => {
                let matrix_location = self
                    .aggregates
                    .projects
                    .get(&project)
                    .expect("Project missing")
                    .matrix
                    .location();
                let presenter = LauncherPresenter::new(
                    matrix_location,
                    id,
                    placement,
                    profile,
                    massive_geometry::Size::default(),
                    scene,
                    &mut self.fonts.lock(),
                );
                self.aggregates.launchers.insert(id, presenter)?;

                self.aggregates
                    .hierarchy
                    .add(DesktopTarget::ProjectMatrix(project), id.into())?;
                DesktopTarget::ProjectMatrix(project).into()
            }
            ProjectCommand::RemoveLauncher(id) => {
                let target = DesktopTarget::Launcher(id);
                let effects = self.remove_target(&target)?;

                self.aggregates.launchers.remove(&id)?;
                effects
            }
            ProjectCommand::SetStartupProfile(launch_profile_id) => {
                self.aggregates.startup_profile = launch_profile_id;
                MeasureSet::Empty
            }
        };

        Ok(measure_set)
    }
}
