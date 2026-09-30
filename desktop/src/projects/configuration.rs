//! The desktop configuration aggregate: projects with matrix-placed launchers,
//! each launcher a spawnable profile, and the profile the session boots into.

use std::ops::Index;

use anyhow::{Context, Result, bail, ensure};
use derive_more::{From, Into};
use serde_json::{Map, Value};
use uuid::Uuid;

use massive_applications::MoveDirection;
use massive_util::CollectingVec;

use crate::desktop_system::ProjectCommand;

/// Spawn parameters of a launcher: JSON values passed to the spawned application.
pub type Params = Map<String, Value>;

/// The canonical desktop configuration: what is parsed from KDL and what the boot
/// flow derives its commands from. Ids are fresh per session and the document is
/// keyed by name, so the two representations only meet during parse and persist.
#[derive(Debug)]
pub struct DesktopConfiguration {
    /// Document order is preserved, so boot commands build the scene like the
    /// document reads.
    projects: Vec<Project>,
    startup: Option<LaunchProfileId>,
}

impl DesktopConfiguration {
    /// Builds the aggregate from parsed projects and the named startup launcher.
    ///
    /// Fails when no launcher exists at all: the session boots into a
    /// configuration-defined launcher, so an empty configuration has nothing to
    /// start.
    pub(super) fn new(projects: Vec<Project>, startup: Option<&str>) -> Result<Self> {
        let empty = projects.iter().all(|project| project.launchers.is_empty());
        ensure!(!empty, "Configuration must define at least one launcher");

        let startup = match startup {
            Some(name) => {
                let id = find_launcher_by_name(&projects, name).with_context(|| {
                    format!("Startup profile '{name}' not found in configuration")
                })?;
                Some(id)
            }
            None => None,
        };

        Ok(Self { projects, startup })
    }

    /// The configuration's projects, in document order.
    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    // --- Mutation API ---
    //
    // Apply-side changes land on the aggregate here so the live model and the
    // plan/apply logic read positions from one source instead of `MatrixPositions`.
    // Parse keeps its own construction path (`new`), where placements are absent
    // from the document and a startup name has not been resolved to an id yet.

    /// Adds a project, or accepts one already present: the boot flow parses the
    /// aggregate and then re-applies the same ids as commands (ids are created at
    /// parse, so the id equality carries the "is already applied" fact).
    pub fn add_project(&mut self, id: ProjectId, name: String) {
        if let Some(project) = self.project_mut(id) {
            project.name = name;
            return;
        }
        self.projects.push(Project {
            id,
            name,
            launchers: Vec::new(),
        });
    }

    pub fn remove_project(&mut self, id: ProjectId) {
        self.projects.retain(|project| project.id != id);
    }

    // A duplicate id is re-application — the boot Setup transaction re-applies
    // commands derived from this very aggregate — so it is ignored, not rejected.
    // A duplicate placement is a fresh launcher on a taken slot: both are then
    // present and the sort order between them stays document-arrival order.
    pub fn add_launcher(
        &mut self,
        project: ProjectId,
        id: LaunchProfileId,
        profile: LaunchProfile,
        placement: MatrixPlacement,
    ) {
        let Some(project) = self.project_mut(project) else {
            return;
        };
        if project.launchers.iter().any(|launcher| launcher.id == id) {
            return;
        }
        let launcher = Launcher {
            id,
            name: profile.name,
            mode: profile.mode,
            params: profile.params,
            placement,
        };
        let index = project.launchers.partition_point(|existing| {
            (existing.placement.column, existing.placement.row) < (placement.column, placement.row)
        });
        project.launchers.insert(index, launcher);
    }

    pub fn move_launcher(&mut self, launcher: LaunchProfileId, to: MatrixPlacement) {
        let Some(existing) = self.launcher_mut(launcher) else {
            return;
        };
        existing.placement = to;
        if let Some(project) = self.project_of_launcher_mut(launcher) {
            project
                .launchers
                .sort_by_key(|launcher| (launcher.placement.column, launcher.placement.row));
        }
    }

    pub fn remove_launcher(&mut self, launcher: LaunchProfileId) {
        if let Some(project) = self.project_of_launcher_mut(launcher) {
            project.launchers.retain(|existing| existing.id != launcher);
        }
    }

    // --- Query API ---
    //
    // These replace what `MatrixPositions` exposed: placement lookup, occupancy
    // checks, and the two shift-sequence computations. They all read the same
    // `launchers` vec that is kept sorted by placement, so the former map and the
    // vec never disagree.

    pub fn project(&self, id: ProjectId) -> Option<&Project> {
        self.projects.iter().find(|project| project.id == id)
    }

    pub fn project_mut(&mut self, id: ProjectId) -> Option<&mut Project> {
        self.projects.iter_mut().find(|project| project.id == id)
    }

    /// The project a launcher belongs to, by searching each project's launchers.
    /// The topology's `project_of_launcher` answers from parent links instead,
    /// and is the right source when the scene hierarchy is available.
    pub fn project_of_launcher(&self, launcher: LaunchProfileId) -> Option<&Project> {
        self.projects
            .iter()
            .find(|project| project.launchers.iter().any(|l| l.id == launcher))
    }

    pub fn project_of_launcher_mut(&mut self, launcher: LaunchProfileId) -> Option<&mut Project> {
        self.projects
            .iter_mut()
            .find(|project| project.launchers.iter().any(|l| l.id == launcher))
    }

    pub fn launcher_mut(&mut self, launcher: LaunchProfileId) -> Option<&mut Launcher> {
        self.project_of_launcher_mut(launcher)
            .and_then(|project| project.launcher_mut(launcher))
    }

    /// The document-order index of the project, which is what orders the projects
    /// on screen: `RemoveProject` by name removes the nearest one.
    pub fn project_index(&self, project: ProjectId) -> Option<usize> {
        self.projects.iter().position(|p| p.id == project)
    }

    /// The placement index of the launcher within its project, counting the
    /// placement-sorted launchers: Manhattan distance between those indexes is the
    /// matrix distance `RemoveLauncher` by name picks the nearest launcher by.
    pub fn launcher_index(&self, launcher: LaunchProfileId) -> Option<usize> {
        let project = self.project_of_launcher(launcher)?;
        project.launchers.iter().position(|l| l.id == launcher)
    }

    /// The launcher's placement, `None` when it is not in the configuration.
    pub fn placement_of(&self, launcher: LaunchProfileId) -> Option<MatrixPlacement> {
        self.launcher(launcher).map(|launcher| launcher.placement)
    }

    /// All launchers of the project, kept in matrix-placement order (the
    /// aggregate invariant).
    pub fn launchers_ordered(&self, project: ProjectId) -> &[Launcher] {
        self.project(project)
            .map(|project| project.launchers.as_slice())
            .unwrap_or(&[])
    }

    pub fn launcher(&self, launcher: LaunchProfileId) -> Option<&Launcher> {
        self.project_of_launcher(launcher)
            .and_then(|project| project.launcher(launcher))
    }

    /// The launcher the session boots into: the startup launcher, or the first
    /// launcher of the projects when no `startup` node names one. `None` is
    /// unreachable for a parsed configuration, whose parse guarantees at least
    /// one launcher.
    pub fn boot_launcher(&self) -> Option<LaunchProfileId> {
        self.startup.or_else(|| {
            self.projects
                .iter()
                .flat_map(|project| project.launchers.iter())
                .next()
                .map(|launcher| launcher.id)
        })
    }

    /// Names a new project: a default name gets the lowest index not already in
    /// use, while a user-chosen name is taken as it is — duplicate names are
    /// allowed. An id the configuration already holds is the boot flow re-applying
    /// the names it parsed, which must pass through unchanged.
    pub fn new_project_name(&self, id: ProjectId, default_name: &str, name: &str) -> String {
        if name != default_name || self.project(id).is_some() {
            return name.to_string();
        }
        let existing: Vec<&str> = self
            .projects
            .iter()
            .map(|project| project.name.as_str())
            .collect();
        indexed_default_name(name, &existing)
    }

    /// The launcher counterpart of [`Self::new_project_name`], indexed among the
    /// siblings of the launcher's project.
    pub fn new_launcher_name(&self, id: LaunchProfileId, default_name: &str, name: &str) -> String {
        if name != default_name || self.launcher(id).is_some() {
            return name.to_string();
        }
        let existing: Vec<&str> = self
            .project_of_launcher(id)
            .map(|project| {
                project
                    .launchers
                    .iter()
                    .map(|launcher| launcher.name.as_str())
                    .collect()
            })
            .unwrap_or_default();
        indexed_default_name(name, &existing)
    }

    /// The project called `name` that sits nearest — in document order — to the
    /// focused project. Duplicate names address the nearest, and with no focused
    /// project or name in it the first match answers; `None` when no project is
    /// so named.
    pub fn nearest_project(&self, name: &str, focused: Option<ProjectId>) -> Option<ProjectId> {
        let focused = focused.and_then(|project| self.project_index(project));
        self.projects
            .iter()
            .enumerate()
            .filter(|(_, project)| project.name == name)
            .map(|(index, project)| (index.abs_diff(focused.unwrap_or(index)), project.id))
            .min_by_key(|(distance, _)| *distance)
            .map(|(_, id)| id)
    }

    /// The launcher called `name` in `project` that sits nearest — in matrix
    /// distance — to `focused`. Duplicate names address the nearest, and with no
    /// focused launcher in `project` the first match answers; `None` when no
    /// launcher of `project` is so named.
    pub fn nearest_launcher(
        &self,
        project: ProjectId,
        name: &str,
        focused: Option<LaunchProfileId>,
    ) -> Option<LaunchProfileId> {
        let focused = focused
            .filter(|launcher| {
                self.project_of_launcher(*launcher)
                    .is_some_and(|owning| owning.id == project)
            })
            .and_then(|launcher| self.launcher_index(launcher))
            .unwrap_or(0);
        self.launchers_ordered(project)
            .iter()
            .enumerate()
            .filter(|(_, launcher)| launcher.name == name)
            .map(|(index, launcher)| (index.abs_diff(focused), launcher.id))
            .min_by_key(|(distance, _)| *distance)
            .map(|(_, id)| id)
    }

    /// How many launchers the configuration defines across all projects.
    pub fn launcher_count(&self) -> usize {
        self.projects
            .iter()
            .map(|project| project.launchers.len())
            .sum()
    }

    /// The launcher occupying `placement` in `project`, if any. Placements are
    /// unique per project, so this is a lookup, not a search over ties.
    pub fn launcher_at(
        &self,
        project: ProjectId,
        placement: MatrixPlacement,
    ) -> Option<LaunchProfileId> {
        self.project(project)
            .and_then(|project| project.launcher_at(placement))
    }

    /// The launchers of `project` that must move for `launcher` to step
    /// `direction`: the contiguous run of occupants ahead of it, listed so that
    /// applying the moves back-to-front stays conflict-free.
    ///
    /// Occupancy is answered from placements alone (each project's launchers are
    /// sorted by placement, and placements are unique per project), so this
    /// matches what `MatrixPositions::shifted_launchers` computed over the same
    /// data.
    pub fn shifted_launchers(
        &self,
        project: ProjectId,
        launcher: LaunchProfileId,
        direction: MoveDirection,
    ) -> Result<Vec<(LaunchProfileId, MatrixPlacement)>> {
        let placement = self
            .placement_of(launcher)
            .with_context(|| format!("launcher {launcher:?} has no matrix placement"))?;

        let mut run = vec![(launcher, placement)];
        loop {
            let (_, leading) = *run.last().expect("run is never empty");
            let Some(next) = leading.moved_placement(direction) else {
                bail!("Can't shift launcher beyond the matrix boundary");
            };
            let Some(next_launcher) = self.launcher_at(project, next) else {
                break;
            };
            run.push((next_launcher, next));
        }

        Ok(run
            .into_iter()
            .rev()
            .map(|(id, moved)| {
                let placement = moved
                    .moved_placement(direction)
                    .expect("run placement was validated before shifting");
                (id, placement)
            })
            .collect())
    }

    /// The launchers right of `placement` in the same row, each moved one column
    /// left — the shift emitted when a slot is freed by launcher removal.
    pub fn shifted_left_launchers(
        &self,
        project: ProjectId,
        placement: MatrixPlacement,
    ) -> Vec<(LaunchProfileId, MatrixPlacement)> {
        self.launchers_ordered(project)
            .iter()
            .filter(|launcher| {
                launcher.placement.row == placement.row
                    && launcher.placement.column > placement.column
            })
            .map(|launcher| {
                (
                    launcher.id,
                    MatrixPlacement {
                        column: launcher.placement.column - 1,
                        row: launcher.placement.row,
                    },
                )
            })
            .collect()
    }
}

/// Resolves the startup launcher's id by name among all launchers.
fn find_launcher_by_name(projects: &[Project], name: &str) -> Option<LaunchProfileId> {
    projects
        .iter()
        .flat_map(|project| project.launchers.iter())
        .find(|launcher| launcher.name == name)
        .map(|launcher| launcher.id)
}

/// The boot commands that rebuild the scene from the aggregate: one per
/// project and launcher, in document order.
pub fn to_commands(configuration: &DesktopConfiguration) -> CollectingVec<ProjectCommand> {
    let mut commands = CollectingVec::Empty;

    commands.push(ProjectCommand::SetStartupLauncher(configuration.startup));

    for project in &configuration.projects {
        project_commands(project, &mut commands);
    }

    commands
}

fn project_commands(project: &Project, commands: &mut CollectingVec<ProjectCommand>) {
    commands.push(ProjectCommand::AddProject {
        id: project.id,
        name: project.name.clone(),
        after: None,
    });

    for launcher in project.launchers() {
        launcher_commands(project.id, launcher, commands);
    }
}

fn launcher_commands(
    project: ProjectId,
    launcher: &Launcher,
    commands: &mut CollectingVec<ProjectCommand>,
) {
    commands.push(ProjectCommand::AddLauncher {
        project,
        id: launcher.id,
        profile: launcher.profile(),
        placement: launcher.matrix_placement(),
    });
}

/// The default name with the lowest index that is not already taken among
/// `existing`. The index only disambiguates the default name; the number is
/// reused once a previous holder is renamed or removed.
fn indexed_default_name(name: &str, existing: &[&str]) -> String {
    let mut index = 2;
    loop {
        let candidate = format!("{name} {index}");
        if !existing.contains(&candidate.as_str()) {
            return candidate;
        }
        index += 1;
    }
}

#[derive(Debug)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    launchers: Vec<Launcher>,
}

impl Project {
    /// Creates the project's id and puts its launchers into sorted order.
    ///
    /// The sort is stable, so launchers sharing a placement keep their document
    /// order.
    pub(crate) fn new(name: String, launchers: Vec<Launcher>) -> Self {
        // Invariant: launchers stay sorted by matrix placement, so consumers can
        // rely on order without re-reading placements.
        let mut launchers = launchers;
        launchers.sort_by_key(|launcher| (launcher.placement.column, launcher.placement.row));
        Self {
            id: ProjectId::new(),
            name,
            launchers,
        }
    }

    pub fn launchers(&self) -> &[Launcher] {
        &self.launchers
    }

    pub fn launcher(&self, id: LaunchProfileId) -> Option<&Launcher> {
        self.launchers.iter().find(|launcher| launcher.id == id)
    }

    pub fn launcher_mut(&mut self, id: LaunchProfileId) -> Option<&mut Launcher> {
        self.launchers.iter_mut().find(|launcher| launcher.id == id)
    }

    /// The launcher occupying `placement`, if any.
    pub fn launcher_at(&self, placement: MatrixPlacement) -> Option<LaunchProfileId> {
        self.launchers
            .iter()
            .find(|launcher| launcher.placement == placement)
            .map(|launcher| launcher.id)
    }
}

/// The aggregate keeps launchers sorted by placement, so readers index by id
/// without caring where the launcher sits in the vec.
impl Index<ProjectId> for DesktopConfiguration {
    type Output = Project;

    fn index(&self, id: ProjectId) -> &Project {
        self.project(id)
            .unwrap_or_else(|| panic!("Project {id:?} is not in the configuration"))
    }
}

impl Index<LaunchProfileId> for DesktopConfiguration {
    type Output = Launcher;

    fn index(&self, id: LaunchProfileId) -> &Launcher {
        self.launcher(id)
            .unwrap_or_else(|| panic!("Launcher {id:?} is not in the configuration"))
    }
}
#[derive(Debug)]
pub struct Launcher {
    pub id: LaunchProfileId,
    pub name: String,
    pub mode: LauncherMode,
    pub params: Params,
    placement: MatrixPlacement,
}

impl Launcher {
    /// Creates the launcher's id.
    pub(crate) fn new(
        name: String,
        mode: LauncherMode,
        params: Params,
        placement: MatrixPlacement,
    ) -> Self {
        Self {
            id: LaunchProfileId::new(),
            name,
            mode,
            params,
            placement,
        }
    }

    /// The launcher's matrix placement, `matrix_` to distinguish it from the
    /// layout `Placement` other types carry under the same name.
    pub fn matrix_placement(&self) -> MatrixPlacement {
        self.placement
    }

    /// The launcher's profile as carried by `AddLauncher` changes.
    pub fn profile(&self) -> LaunchProfile {
        LaunchProfile {
            name: self.name.clone(),
            mode: self.mode,
            params: self.params.clone(),
        }
    }
}

/// The spawnable profile of a launcher, carried by changes and shown by the
/// `LauncherPresenter`.
#[derive(Debug, Clone)]
pub struct LaunchProfile {
    pub name: String,
    pub mode: LauncherMode,
    pub params: Params,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct MatrixPlacement {
    pub column: u32,
    pub row: u32,
}

impl MatrixPlacement {
    /// The placement one `direction` step away, `None` past the matrix edge.
    pub fn moved_placement(self, direction: MoveDirection) -> Option<MatrixPlacement> {
        match direction {
            MoveDirection::Left => self.column.checked_sub(1).map(|column| MatrixPlacement {
                column,
                row: self.row,
            }),
            MoveDirection::Right => self.column.checked_add(1).map(|column| MatrixPlacement {
                column,
                row: self.row,
            }),
            MoveDirection::Up => self.row.checked_sub(1).map(|row| MatrixPlacement {
                row,
                column: self.column,
            }),
            MoveDirection::Down => self.row.checked_add(1).map(|row| MatrixPlacement {
                row,
                column: self.column,
            }),
        }
    }
}

impl From<(u32, u32)> for MatrixPlacement {
    fn from((column, row): (u32, u32)) -> Self {
        Self { column, row }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From, Into)]
pub struct ProjectId(Uuid);

impl ProjectId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From, Into)]
pub struct LaunchProfileId(Uuid);

impl LaunchProfileId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum LauncherMode {
    Band,
    #[default]
    Visor,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launcher(id: LaunchProfileId, placement: MatrixPlacement) -> Launcher {
        Launcher {
            id,
            name: format!("{id:?}"),
            mode: LauncherMode::Visor,
            params: Default::default(),
            placement,
        }
    }

    fn configuration(
        entries: impl IntoIterator<Item = (LaunchProfileId, MatrixPlacement)>,
    ) -> DesktopConfiguration {
        let entries: Vec<_> = entries.into_iter().collect();
        let project = Project::new(
            "only".into(),
            entries
                .into_iter()
                .map(|(id, placement)| launcher(id, placement))
                .collect(),
        );
        DesktopConfiguration {
            projects: vec![project],
            startup: None,
        }
    }

    // Ported from `MatrixPositions::shifted_launchers` tests when the aggregate
    // became the one source of matrix placements: these pin the shift-sequence
    // semantics (reverse order, stop at gaps, boundary errors).
    #[test]
    fn shifted_launchers_moves_a_contiguous_run_in_reverse_order() {
        let first = LaunchProfileId::new();
        let second = LaunchProfileId::new();
        let third = LaunchProfileId::new();
        let configuration = configuration([
            (first, MatrixPlacement { column: 1, row: 0 }),
            (second, MatrixPlacement { column: 2, row: 0 }),
            (third, MatrixPlacement { column: 3, row: 0 }),
        ]);

        let shifted = configuration
            .shifted_launchers(project_id_of(&configuration), first, MoveDirection::Right)
            .unwrap();

        assert_eq!(
            shifted,
            vec![
                (third, MatrixPlacement { column: 4, row: 0 }),
                (second, MatrixPlacement { column: 3, row: 0 }),
                (first, MatrixPlacement { column: 2, row: 0 }),
            ]
        );
    }

    #[test]
    fn shifted_launchers_stops_at_the_first_empty_slot() {
        let first = LaunchProfileId::new();
        let second = LaunchProfileId::new();
        let configuration = configuration([
            (first, MatrixPlacement { column: 1, row: 0 }),
            (second, MatrixPlacement { column: 3, row: 0 }),
        ]);

        let shifted = configuration
            .shifted_launchers(project_id_of(&configuration), first, MoveDirection::Right)
            .unwrap();

        assert_eq!(
            shifted,
            vec![(first, MatrixPlacement { column: 2, row: 0 })]
        );
    }

    #[test]
    fn shifted_launchers_rejects_left_and_up_boundaries() {
        let launcher = LaunchProfileId::new();
        let configuration = configuration([(launcher, MatrixPlacement { column: 0, row: 0 })]);

        assert!(
            configuration
                .shifted_launchers(project_id_of(&configuration), launcher, MoveDirection::Left)
                .is_err()
        );
        assert!(
            configuration
                .shifted_launchers(project_id_of(&configuration), launcher, MoveDirection::Up)
                .is_err()
        );
    }

    fn project_id_of(configuration: &DesktopConfiguration) -> ProjectId {
        configuration.projects[0].id
    }
}
