//! The desktop configuration aggregate: projects whose matrices hold slots, each
//! slot a launcher or a nested project, and the launcher the session boots into.
//!
//! Projects are stored flat and nest by id link (ADR 0011): a project's slots
//! name the launchers and projects they host, so no recursive type is needed.

use std::collections::HashSet;
use std::ops::Index;

use anyhow::{Context, Result, bail, ensure};
use derive_more::{From, Into};
use indexmap::IndexMap;
use log::warn;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use massive_applications::MoveDirection;
use massive_util::CollectingVec;

use crate::desktop_system::{DesktopTarget, ProjectCommand, SlotShift};

/// Spawn parameters of a launcher: JSON values passed to the spawned application.
pub type Params = Map<String, Value>;

/// The name the terminal gives the root project it synthesizes.
pub const ROOT_PROJECT_NAME: &str = "Projects";

/// The default name a project created by an assignment takes when the assignment
/// names none; a taken name is indexed (see [`DesktopConfiguration::new_project_name`]).
pub const DEFAULT_NEW_PROJECT_NAME: &str = "New Project";

/// The default name a launcher created by an assignment takes. The parse also
/// gives a launcher-less configuration a launcher under this name (ADR 0012).
pub const DEFAULT_NEW_LAUNCHER_NAME: &str = "New Launcher";

/// The launcher presentation a slot's panel uses. Serialized as the lowercase
/// variant name, `visor` the default.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LauncherMode {
    Band,
    #[default]
    Visor,
}

/// The Full Screen Mode shared by a launcher's base instances (ADR 0014).
/// Serialized as the lowercase variant name, `regular` the default.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FullScreenMode {
    #[default]
    Regular,
    FullScreen,
}

impl FullScreenMode {
    pub fn toggled(self) -> Self {
        match self {
            Self::Regular => Self::FullScreen,
            Self::FullScreen => Self::Regular,
        }
    }
}

/// The canonical desktop configuration: what is parsed from the configuration
/// document and what the boot flow derives its commands from. Ids are fresh per
/// session and the document is keyed by name, so the two representations only
/// meet during parse and persist.
///
/// The document is the JSON file the configuration persists to
/// ([`crate::projects::persistence::PersistedConfiguration`]); it is derived from this
/// aggregate when persisted (ADR 0013).
#[derive(Debug)]
pub struct RuntimeConfiguration {
    /// Insertion order is the document order, so boot commands build the scene
    /// like the document reads; ids are unique, which the map's key type makes
    /// structural rather than a discipline every call site re-checks.
    projects: IndexMap<ProjectId, Project>,
    startup: Option<LaunchProfileId>,
    startup_path: Option<String>,
}

impl RuntimeConfiguration {
    /// Builds the aggregate from the parsed projects. The root project (`Projects`,
    /// id [`ProjectId::ROOT`]) comes with the parsed slots; the boot command
    /// re-applies it (see [`DesktopConfiguration::add_project`]).
    ///
    /// A configuration whose launchers all sit in nested projects is legal, and so
    /// is one with no launcher anywhere: a separate step adds a launcher to the
    /// root when the file defines none (ADR 0012).
    pub(super) fn new(projects: Vec<Project>, startup: Option<&str>) -> Self {
        let projects: IndexMap<ProjectId, Project> = projects
            .into_iter()
            .map(|project| (project.id, project))
            .collect();
        let mut configuration = Self {
            projects,
            startup: None,
            startup_path: startup.map(str::to_owned),
        };

        configuration.startup = startup.and_then(|path| {
            match configuration.resolve_launcher_path(ProjectId::ROOT, path) {
                Some(launcher) => Some(launcher),
                None => {
                    warn!("Startup '{path}' does not resolve; falling back to the first launcher");
                    configuration.first_launcher_depth_first()
                }
            }
        });

        configuration
    }

    /// The nested project `project` hosts at `placement`, if any.
    #[cfg(test)]
    pub fn child_project_at(
        &self,
        project: ProjectId,
        placement: MatrixPlacement,
    ) -> Option<ProjectId> {
        match self.project(project)?.content_at(placement)? {
            SlotContent::Project(child) => Some(child),
            SlotContent::Launcher(_) => None,
        }
    }

    /// The configuration's projects, in document order. Test-only: the tests
    /// assert against the parsed aggregate, while production reads go through
    /// the `project`/`launchers_ordered` lookups.
    #[cfg(test)]
    pub fn projects(&self) -> impl ExactSizeIterator<Item = &Project> {
        self.projects.values()
    }

    // --- Mutation API ---
    //
    // Apply-side changes land on the aggregate here so the live model and the
    // plan/apply logic read slot assignment from one source. Parse keeps its own
    // construction path (`new`), where placements come from the document and a
    // startup path has not been resolved to an id yet.

    /// Removes the project and every project nested below it: a project cannot
    /// outlive the slot hosting it. The root is not removable.
    ///
    /// Removing a project's last slot does not remove the project — empty projects
    /// are legal, and only this explicit removal cascades.
    #[cfg(test)]
    pub fn remove_project(&mut self, id: ProjectId) {
        if id == ProjectId::ROOT {
            return;
        }
        let doomed: Vec<ProjectId> = self
            .projects
            .values()
            .map(|project| project.id)
            .filter(|candidate| *candidate == id || self.is_in_subtree(id, *candidate))
            .collect();

        for project in self.projects.values_mut() {
            project.slots.retain(
                |slot| !matches!(slot.content(), SlotContent::Project(child) if doomed.contains(&child)),
            );
        }

        self.projects
            .retain(|project_id, _| !doomed.contains(project_id));
    }

    /// Assigns `content` to `parent`'s slot at `placement`, returning the host
    /// the content was assigned into — `None` when the assignment created a
    /// hostless project (the root).
    ///
    /// Assigning a project that does not exist creates it — there is no separate
    /// "add project" operation. A `None` parent creates the root the same way:
    /// the root is hosted by the `Desktop` target, not a matrix, so nothing is
    /// slotted. Replacing content is a clear plus an assign, so the caller clears
    /// the slot first.
    pub fn assign_slot(
        &mut self,
        parent: Option<ProjectId>,
        placement: MatrixPlacement,
        content: SlotAssignment,
    ) -> Option<ProjectId> {
        match content {
            SlotAssignment::Launcher { id, profile } => {
                let parent = parent.expect("a launcher is always assigned into a project's slot");
                let launcher = Launcher {
                    id,
                    name: profile.name.clone(),
                    mode: profile.mode,
                    params: profile.params.clone(),
                    full_screen_mode: profile.full_screen_mode,
                };
                let project = self
                    .project_mut(parent)
                    .expect("the parent project of a launcher exists");
                project.assign_slot(Slot::launcher(placement, launcher));
                Some(parent)
            }
            SlotAssignment::Project { id, name } => {
                // A duplicate id is re-application — the boot Setup transaction
                // re-applies commands derived from this very aggregate — so it is
                // ignored, not rejected.
                if self.project(id).is_none() {
                    let name = self.new_project_name(id, DEFAULT_NEW_PROJECT_NAME, &name);
                    self.add_project(id, name);
                }
                if let Some(parent) = parent {
                    self.project_mut(parent)
                        .expect("the parent project of the assignment exists")
                        .assign_slot(Slot::project(placement, id));
                }
                parent
            }
        }
    }

    /// Adds a project with no slots. A duplicate id never reaches here: the
    /// caller ignores it as re-application. Insertion keeps the document order
    /// the IndexMap records.
    pub fn add_project(&mut self, id: ProjectId, name: String) {
        let replaced = self.projects.insert(
            id,
            Project {
                id,
                name,
                slots: Vec::new(),
            },
        );
        debug_assert!(replaced.is_none(), "duplicate project id {id:?}");
    }

    /// Empties `project`'s slot at `placement`. A slot that hosts no content is
    /// simply absent, so clearing an already empty slot does nothing.
    pub fn clear_slot(&mut self, project: ProjectId, placement: MatrixPlacement) {
        self.project_mut(project)
            .expect("the project hosting the slot exists")
            .remove_slot(placement);
    }

    /// Moves the content of `source` to `dest`, returning whether the move
    /// happened.
    ///
    /// Rejected — with no change emitted — when the source slot is empty, when the
    /// destination is inside the moved project's own subtree, or when the
    /// destination project is not in the tree.
    pub fn move_slot(
        &mut self,
        source: (ProjectId, MatrixPlacement),
        dest: (ProjectId, MatrixPlacement),
    ) -> bool {
        let (source_project, source_placement) = source;
        let (dest_project, dest_placement) = dest;

        if source == dest || self.project(dest_project).is_none() {
            return false;
        }
        let Some(content) = self.content_at(source_project, source_placement) else {
            return false;
        };
        if let SlotContent::Project(moved) = content
            && self.is_in_subtree(moved, dest_project)
        {
            return false;
        }

        let Some(index) = self
            .project(source_project)
            .and_then(|project| project.slot_index(source_placement))
        else {
            return false;
        };
        let Some(source_project_mut) = self.project_mut(source_project) else {
            return false;
        };
        let mut slot = source_project_mut.slots.remove(index);
        // The destination and the removed slot are distinct placements, so
        // re-adding under the new placement cannot collide with the source slot.
        slot.placement = dest_placement;
        self.project_mut(dest_project)
            .expect("the destination project was checked above")
            .assign_slot(slot);
        true
    }

    /// Whether the content of `source` can move to `dest`.
    ///
    /// Rejected when the source slot is empty, when the destination project is not
    /// in the tree, and when the destination is inside the moved project's own
    /// subtree — a project cannot be moved into itself.
    pub fn can_move_slot(
        &self,
        source: (ProjectId, MatrixPlacement),
        dest: (ProjectId, MatrixPlacement),
    ) -> bool {
        let (source_project, source_placement) = source;
        let (dest_project, _) = dest;

        if source == dest || self.project(dest_project).is_none() {
            return false;
        }
        let Some(content) = self.content_at(source_project, source_placement) else {
            return false;
        };
        if let SlotContent::Project(moved) = content
            && self.is_in_subtree(moved, dest_project)
        {
            return false;
        }
        true
    }

    // --- Query API ---
    //
    // Placement lookup, assignment checks, and the two shift-sequence computations.

    /// Whether `candidate` is `project` or nested below it, following slot links.
    pub fn is_in_subtree(&self, project: ProjectId, candidate: ProjectId) -> bool {
        let mut pending = vec![project];
        let mut visited = HashSet::new();
        while let Some(current) = pending.pop() {
            if !visited.insert(current) {
                continue;
            }
            if current == candidate {
                return true;
            }
            if let Some(project) = self.project(current) {
                pending.extend(project.nested_projects());
            }
        }
        false
    }

    /// The launcher the session boots into: the startup launcher, or the first
    /// launcher of the tree when no `startup` node names one. `None` is
    /// unreachable for a loaded configuration: the load gives a launcher-less one
    /// a launcher (ADR 0012).
    pub fn boot_launcher(&self) -> Option<LaunchProfileId> {
        self.startup.or_else(|| self.first_launcher_depth_first())
    }

    /// The startup address path as the configuration was loaded with, `None`
    /// when no startup launcher is set. The path is what persists; the resolved
    /// launcher id is fresh per session and not serialized.
    pub fn startup_path(&self) -> Option<&str> {
        self.startup_path.as_deref()
    }

    /// Records the startup address path. The resolved launcher id is left as it
    /// is: the path is what the document carries, and the id is re-resolved at
    /// the next load.
    pub fn set_startup_path(&mut self, path: Option<String>) {
        self.startup_path = path;
    }

    /// The first launcher of a depth-first walk from the root — the fallback the
    /// startup path and `boot_launcher` share, so a root whose slots are all
    /// nested projects still boots.
    pub fn first_launcher_depth_first(&self) -> Option<LaunchProfileId> {
        let mut current = ProjectId::ROOT;
        loop {
            if let Some(launcher_id) = self.project(current).and_then(|project| {
                project.slots().iter().find_map(|slot| match &slot.content {
                    SlotPayload::Launcher(launcher) => Some(launcher.id),
                    SlotPayload::Project(_) => None,
                })
            }) {
                return Some(launcher_id);
            }
            let next = self
                .project(current)
                .and_then(|project| project.first_nested_project())?;
            current = next;
        }
    }

    pub(crate) fn launcher_address_path(&self, launcher: LaunchProfileId) -> Result<String> {
        let launcher_record = self
            .launcher(launcher)
            .with_context(|| format!("launcher {launcher:?} is not in the configuration"))?;
        let mut segments = vec![launcher_record.name.clone()];
        let mut project = self
            .project_of_launcher(launcher)
            .context("launcher has no parent project")?
            .id;

        while project != ProjectId::ROOT {
            let record = self
                .project(project)
                .with_context(|| format!("project {project:?} is not in the configuration"))?;
            segments.push(record.name.clone());
            project = self
                .projects
                .values()
                .find(|candidate| candidate.nested_projects().contains(&record.id))
                .context("nested project has no parent")?
                .id;
        }

        ensure!(
            segments
                .iter()
                .all(|segment| !segment.is_empty() && !segment.contains('/')),
            "startup address path cannot represent a name containing '/'"
        );
        segments.reverse();
        let path = format!("/{}", segments.join("/"));
        ensure!(
            self.resolve_launcher_path(ProjectId::ROOT, &path) == Some(launcher),
            "startup address path does not resolve to the selected launcher"
        );
        Ok(path)
    }

    /// Resolves a startup address path to a launcher id.
    ///
    /// The last segment names the launcher and the segments before it walk the
    /// project tree; each segment takes the nearest match, so a path that misses
    /// at one level still resolves within the nearest project. A leading
    /// separator addresses from the root (the root itself has no name, so the
    /// first segment is the first root slot); otherwise the path is relative to
    /// `base`.
    pub fn resolve_launcher_path(&self, base: ProjectId, path: &str) -> Option<LaunchProfileId> {
        let (project, last) = self.resolve_path(base, path)?;
        self.project(project)
            .and_then(|project| project.launcher_by_name(last))
    }

    /// Resolves an address path to a project, `None` when no segment resolves.
    ///
    /// The path names the project itself, so `labs` resolves to the project
    /// called `labs` and `labs/shell` resolves to the project `labs` — see
    /// [`Self::resolve_launcher_path`] for the launcher counterpart.
    pub fn resolve_project_path(&self, base: ProjectId, path: &str) -> Option<ProjectId> {
        let (parent, last) = self.resolve_path(base, path)?;
        self.nearest_child_project(parent, last)
    }

    /// Walks the leading segments of `path`, returning the project the last
    /// segment resolved to and the final segment (the addressed content's name).
    fn resolve_path<'p>(&self, base: ProjectId, path: &'p str) -> Option<(ProjectId, &'p str)> {
        let mut current = if path.starts_with('/') {
            ProjectId::ROOT
        } else {
            base
        };

        let mut last = None;
        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            if let Some(previous) = last {
                // The previous segment addressed a project by name.
                current = self.nearest_child_project(current, previous)?;
            }
            last = Some(segment);
        }

        let last = last.unwrap_or(ROOT_PROJECT_NAME);
        Some((current, last))
    }

    /// The project called `name` nested in `parent`, or `parent` itself when no
    /// nested project is so named.
    fn nearest_child_project(&self, parent: ProjectId, name: &str) -> Option<ProjectId> {
        if self
            .project(parent)
            .is_some_and(|project| project.name == name)
        {
            return Some(parent);
        }
        let children = self
            .project(parent)
            .map(|project| project.nested_projects())
            .unwrap_or_default();
        children
            .iter()
            .find(|child| {
                self.project(**child)
                    .is_some_and(|project| project.name == name)
            })
            .copied()
    }

    /// Names a newly assigned project: a default name gets the lowest index not
    /// already in use, while a user-chosen name is taken as it is — duplicate
    /// names are allowed. An id the configuration already holds is the boot flow
    /// re-applying the names it parsed, which must pass through unchanged.
    pub fn new_project_name(&self, id: ProjectId, default_name: &str, name: &str) -> String {
        if name != default_name || self.project(id).is_some() {
            return name.to_string();
        }
        let existing: Vec<&str> = self
            .projects
            .values()
            .map(|project| project.name.as_str())
            .collect();
        indexed_default_name(name, &existing)
    }

    /// Whether any project's matrix hosts a launcher. The load gives a
    /// launcher-less configuration one, so a loaded configuration answers `true`
    /// (ADR 0012).
    pub fn has_launcher(&self) -> bool {
        self.projects.values().any(|project| project.has_launcher())
    }

    /// All launchers in the configuration, grouped by project document order.
    pub fn launchers(&self) -> impl Iterator<Item = &Launcher> + '_ {
        self.projects
            .values()
            .flat_map(|project| project.launchers())
    }

    /// How many launchers the configuration defines across all projects.
    #[cfg(test)]
    pub fn launcher_count(&self) -> usize {
        self.projects
            .values()
            .map(|project| project.launchers().count())
            .sum()
    }

    /// The launcher assigned to `placement` in `project`, if any. Placements are
    /// unique per project, so this is a lookup, not a search over ties.
    #[cfg(test)]
    pub fn launcher_at(
        &self,
        project: ProjectId,
        placement: MatrixPlacement,
    ) -> Option<LaunchProfileId> {
        self.project(project)
            .and_then(|project| project.launcher_at(placement))
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
            .enumerate()
            .filter(|(_, launcher)| launcher.name == name)
            .map(|(index, launcher)| (index.abs_diff(focused), launcher.id))
            .min_by_key(|(distance, _)| *distance)
            .map(|(_, id)| id)
    }

    /// The project called `name` that sits nearest — in document order — to the
    /// focused project. Duplicate names address the nearest, and with no focused
    /// project or name in it the first match answers; `None` when no project is
    /// so named.
    pub fn nearest_project(&self, name: &str, focused: Option<ProjectId>) -> Option<ProjectId> {
        let focused = focused.and_then(|project| self.project_index(project));
        self.projects
            .values()
            .enumerate()
            .filter(|(_, project)| project.name == name)
            .map(|(index, project)| (index.abs_diff(focused.unwrap_or(index)), project.id))
            .min_by_key(|(distance, _)| *distance)
            .map(|(_, id)| id)
    }

    /// The document-order index of the project, which is what orders the projects
    /// on screen: `RemoveProject` by name removes the nearest one.
    pub fn project_index(&self, project: ProjectId) -> Option<usize> {
        self.projects.get_index_of(&project)
    }

    /// The launchers of `project` that must move for the content at `placement`
    /// to step `direction`: the contiguous run of assigned slots ahead of it,
    /// listed so that applying the moves back-to-front stays conflict-free.
    ///
    /// The shift is content-agnostic per slot — a project-assigned slot shifts
    /// like a launcher-assigned one.
    pub fn shifted_slots(
        &self,
        project: ProjectId,
        placement: MatrixPlacement,
        direction: MoveDirection,
    ) -> Result<Vec<(MatrixPlacement, MatrixPlacement)>> {
        let mut run = vec![placement];
        loop {
            let leading = *run.last().expect("run is never empty");
            let Some(next) = leading.moved_placement(direction) else {
                bail!("Can't shift slot content beyond the matrix boundary");
            };
            if self.content_at(project, next).is_none() {
                break;
            }
            run.push(next);
        }

        Ok(run
            .into_iter()
            .rev()
            .map(|moved| {
                let placement = moved
                    .moved_placement(direction)
                    .expect("run placement was validated before shifting");
                (moved, placement)
            })
            .collect())
    }

    /// The assigned slots right of `placement` in the same row, each moved one
    /// column left — the shift emitted when a slot is freed.
    pub fn shifted_left_slots(
        &self,
        project: ProjectId,
        placement: MatrixPlacement,
    ) -> Vec<(MatrixPlacement, MatrixPlacement)> {
        self.slots_ordered(project)
            .filter(|(candidate, _)| {
                candidate.row == placement.row && candidate.column > placement.column
            })
            .map(|(candidate, _)| {
                (
                    candidate,
                    MatrixPlacement {
                        column: candidate.column - 1,
                        row: candidate.row,
                    },
                )
            })
            .collect()
    }

    /// The placement index of the launcher within its project, counting the launchers in
    /// placement order: Manhattan distance between those indexes is the matrix distance
    /// `RemoveLauncher` by name picks the nearest launcher by.
    pub fn launcher_index(&self, launcher: LaunchProfileId) -> Option<usize> {
        let project = self.project_of_launcher(launcher)?;
        project.launchers().position(|l| l.id == launcher)
    }

    /// The project and placement of the slot holding `content`.
    pub fn slot_of_content(
        &self,
        content: impl Into<SlotContent>,
    ) -> Option<(ProjectId, MatrixPlacement)> {
        let content = content.into();
        self.projects.values().find_map(|project| {
            project
                .placement_of_content(content)
                .map(|placement| (project.id, placement))
        })
    }

    /// The content of `project`'s slot at `placement`, `None` when the slot is
    /// empty. This is the assignment query the shift planner and `AssignSlot`
    /// validation need; placements are unique per project, so it is a lookup.
    pub fn content_at(
        &self,
        project: ProjectId,
        placement: MatrixPlacement,
    ) -> Option<SlotContent> {
        self.project(project)
            .and_then(|project| project.content_at(placement))
    }

    /// All launchers of the project, kept in matrix-placement order (the
    /// aggregate invariant).
    pub fn launchers_ordered(&self, project: ProjectId) -> impl Iterator<Item = &Launcher> + '_ {
        self.project(project)
            .into_iter()
            .flat_map(|project| project.launchers())
    }

    pub fn launcher(&self, launcher: LaunchProfileId) -> Option<&Launcher> {
        self.project_of_launcher(launcher)
            .and_then(|project| project.launcher(launcher))
    }

    /// Mutable access to a launcher record, for Full Screen Mode changes
    /// (ADR 0014). The launcher lives in its slot's payload.
    pub fn launcher_mut(&mut self, launcher: LaunchProfileId) -> Option<&mut Launcher> {
        self.projects
            .values_mut()
            .find_map(|project| project.launcher_mut(launcher))
    }

    /// The project a launcher belongs to, by searching each project's slots. The
    /// topology's `project_of_target` answers from parent links instead, and is
    /// the right source when the scene hierarchy is available.
    pub fn project_of_launcher(&self, launcher: LaunchProfileId) -> Option<&Project> {
        self.slot_of_content(launcher)
            .and_then(|(project, _)| self.project(project))
    }

    /// Every assigned slot of `project`, in placement order. `MatrixPlacement`
    /// and `SlotContent` are both `Copy`, so the items are values without a
    /// collection to build first.
    pub fn slots_ordered(
        &self,
        project: ProjectId,
    ) -> impl Iterator<Item = (MatrixPlacement, SlotContent)> + '_ {
        self.project(project).into_iter().flat_map(|project| {
            project
                .slots
                .iter()
                .map(|slot| (slot.placement, slot.content()))
        })
    }

    pub fn project(&self, id: ProjectId) -> Option<&Project> {
        self.projects.get(&id)
    }

    pub fn project_mut(&mut self, id: ProjectId) -> Option<&mut Project> {
        self.projects.get_mut(&id)
    }
}

/// The boot commands that rebuild the scene from the aggregate: a pre-order walk
/// from the root, so a slot's parent project and matrix exist before the slot's
/// content is assigned under them.
///
/// The walk starts with the root project's own `AssignSlot { parent: None }`,
/// which parents the root project under the `Desktop` target; every other
/// project's creation is its slot assignment.
pub fn to_commands(configuration: &RuntimeConfiguration) -> CollectingVec<ProjectCommand> {
    let mut commands = CollectingVec::Empty;

    commands.push(ProjectCommand::SetStartupPath(
        configuration.startup_path.clone(),
    ));
    let root_name = configuration
        .project(ProjectId::ROOT)
        .map(|project| project.name.clone())
        .unwrap_or_else(|| ROOT_PROJECT_NAME.to_string());
    commands.push(ProjectCommand::AssignSlot {
        parent: None,
        placement: MatrixPlacement { column: 0, row: 0 },
        content: SlotAssignment::Project {
            id: ProjectId::ROOT,
            name: root_name,
        },
        // Ignored for the parentless root creation.
        shift: SlotShift::default(),
    });
    subtree_commands(configuration, ProjectId::ROOT, &mut commands);

    commands
}

/// Emits one `AssignSlot` command per assigned slot of `project`, then recurses
/// into nested projects — the per-project step of the pre-order walk
/// [`to_commands`] performs. Every slot's launcher and every nested project
/// exists by the aggregate's id-link invariant, so a missing one is a broken
/// invariant that panics through the `Index` impls — not a slot to skip.
fn subtree_commands(
    configuration: &RuntimeConfiguration,
    project: ProjectId,
    commands: &mut CollectingVec<ProjectCommand>,
) {
    for slot in configuration[project].slots() {
        let content = match &slot.content {
            SlotPayload::Launcher(launcher) => SlotAssignment::Launcher {
                id: launcher.id,
                profile: launcher.profile(),
            },
            SlotPayload::Project(child) => SlotAssignment::Project {
                id: *child,
                name: configuration[*child].name.clone(),
            },
        };

        commands.push(ProjectCommand::AssignSlot {
            parent: Some(project),
            placement: slot.placement,
            content,
            // Boot re-applies positions the aggregate already holds, so nothing
            // is displaced.
            shift: SlotShift::Keep,
        });

        if let SlotPayload::Project(child) = &slot.content {
            subtree_commands(configuration, *child, commands);
        }
    }
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

/// A project's matrix: its assigned slots.
///
/// The vector is kept sorted by placement (`sort_slots`): the shift computations
/// and nearest-by-name picks read slots in placement order, so readers can rely
/// on order without re-sorting.
#[derive(Debug, Clone)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    slots: Vec<Slot>,
}

impl Project {
    /// Creates the project's id and puts its slots into placement order.
    ///
    /// The sort is stable, so slots sharing a placement keep their document order.
    pub(crate) fn new(name: String, mut slots: Vec<Slot>) -> Self {
        sort_slots(&mut slots);
        Self {
            id: ProjectId::new(),
            name,
            slots,
        }
    }

    /// Puts `slot` into the matrix, replacing whatever content is assigned to
    /// its placement.
    ///
    /// An assign into an assigned slot cannot displace the assigned content —
    /// the shift happens in the plan, before the live model sees the assignment.
    pub(crate) fn assign_slot(&mut self, slot: Slot) {
        if let Some(index) = self.slot_index(slot.placement) {
            self.slots[index] = slot;
            return;
        }
        self.slots.push(slot);
        sort_slots(&mut self.slots);
    }

    /// The launcher assigned to `placement`, if the slot holds one.
    #[cfg(test)]
    pub fn launcher_at(&self, placement: MatrixPlacement) -> Option<LaunchProfileId> {
        match self.content_at(placement)? {
            SlotContent::Launcher(launcher) => Some(launcher),
            SlotContent::Project(_) => None,
        }
    }

    /// The content of the slot at `placement`, `None` when it is empty.
    pub fn content_at(&self, placement: MatrixPlacement) -> Option<SlotContent> {
        self.slot_index(placement)
            .map(|index| self.slots[index].content())
    }

    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    /// Whether the project hosts a launcher anywhere in its own matrix. Nested
    /// projects' launchers are not counted: this answers for one matrix.
    pub fn has_launcher(&self) -> bool {
        self.slots
            .iter()
            .any(|slot| matches!(slot.content, SlotPayload::Launcher(_)))
    }

    /// The launchers of this project's matrix, in placement order.
    pub fn launchers(&self) -> impl Iterator<Item = &Launcher> {
        self.slots.iter().filter_map(|slot| match &slot.content {
            SlotPayload::Launcher(launcher) => Some(launcher),
            SlotPayload::Project(_) => None,
        })
    }

    /// The ids of the projects nested in this project's matrix, in placement order.
    pub fn nested_projects(&self) -> Vec<ProjectId> {
        self.slots
            .iter()
            .map(|slot| slot.content())
            .filter_map(|content| match content {
                SlotContent::Project(project) => Some(project),
                SlotContent::Launcher(_) => None,
            })
            .collect()
    }

    /// The first project nested in this matrix, in placement order — the walk
    /// `first_launcher_depth_first` follows.
    pub fn first_nested_project(&self) -> Option<ProjectId> {
        self.slots.iter().find_map(|slot| match slot.content() {
            SlotContent::Project(project) => Some(project),
            SlotContent::Launcher(_) => None,
        })
    }

    pub fn launcher(&self, id: LaunchProfileId) -> Option<&Launcher> {
        self.slots.iter().find_map(|slot| match &slot.content {
            SlotPayload::Launcher(launcher) if launcher.id == id => Some(launcher),
            _ => None,
        })
    }

    pub fn launcher_mut(&mut self, id: LaunchProfileId) -> Option<&mut Launcher> {
        self.slots
            .iter_mut()
            .find_map(|slot| match &mut slot.content {
                SlotPayload::Launcher(launcher) if launcher.id == id => Some(launcher),
                _ => None,
            })
    }

    /// The placement of the slot whose ids match `content`.
    pub fn placement_of_content(&self, content: impl Into<SlotContent>) -> Option<MatrixPlacement> {
        let content = content.into();
        self.slots
            .iter()
            .find(|slot| slot.content() == content)
            .map(|slot| slot.placement)
    }

    /// The launcher called `name` of this matrix, nearest first in placement
    /// order — duplicates address the first.
    pub fn launcher_by_name(&self, name: &str) -> Option<LaunchProfileId> {
        self.slots.iter().find_map(|slot| match &slot.content {
            SlotPayload::Launcher(launcher) if launcher.name == name => Some(launcher.id),
            _ => None,
        })
    }

    /// Empties the slot at `placement`, if it is assigned.
    pub(crate) fn remove_slot(&mut self, placement: MatrixPlacement) {
        self.slots.retain(|slot| slot.placement != placement);
    }

    /// The index of the slot at `placement`.
    pub fn slot_index(&self, placement: MatrixPlacement) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| slot.placement == placement)
    }
}

/// Puts slots into placement order: row first, then column, matching the order the
/// aggregate's readers rely on.
fn sort_slots(slots: &mut [Slot]) {
    slots.sort_by_key(|slot| (slot.placement.row, slot.placement.column));
}

/// One assigned slot of a project's matrix.
///
/// Slots are implicit (ADR 0011): a slot exists only while it has content, and it
/// is addressed by its `(project, placement)` key rather than by an id. The
/// placement is `Slot`'s own field — the launcher payload carries none — so a
/// moved slot cannot diverge from its content.
#[derive(Debug, Clone)]
pub struct Slot {
    pub placement: MatrixPlacement,
    pub content: SlotPayload,
}

impl Slot {
    /// Builds a launcher-assigned slot, which is how slots are created at parse
    /// and on assign.
    pub(crate) fn launcher(placement: MatrixPlacement, launcher: Launcher) -> Self {
        Self {
            placement,
            content: SlotPayload::Launcher(launcher),
        }
    }

    /// Builds a project-assigned slot.
    pub(crate) fn project(placement: MatrixPlacement, project: ProjectId) -> Self {
        Self {
            placement,
            content: SlotPayload::Project(project),
        }
    }

    /// The content's ids, for readers that only discriminate — the cheap
    /// `Copy` projection of the payload.
    pub fn content(&self) -> SlotContent {
        match &self.content {
            SlotPayload::Launcher(launcher) => SlotContent::Launcher(launcher.id),
            SlotPayload::Project(project) => SlotContent::Project(*project),
        }
    }
}

/// The aggregate keeps a project's slots sorted by placement, so readers index by
/// id without caring where in the vec the content sits.
impl Index<ProjectId> for RuntimeConfiguration {
    type Output = Project;

    fn index(&self, id: ProjectId) -> &Project {
        self.project(id)
            .unwrap_or_else(|| panic!("Project {id:?} is not in the configuration"))
    }
}

impl Index<LaunchProfileId> for RuntimeConfiguration {
    type Output = Launcher;

    fn index(&self, id: LaunchProfileId) -> &Launcher {
        self.launcher(id)
            .unwrap_or_else(|| panic!("Launcher {id:?} is not in the configuration"))
    }
}
#[derive(Debug, Clone)]
pub struct Launcher {
    pub id: LaunchProfileId,
    pub name: String,
    pub mode: LauncherMode,
    pub params: Params,
    /// The Full Screen Mode all of the launcher's base instances follow
    /// (ADR 0014). Persisted with the configuration.
    pub full_screen_mode: FullScreenMode,
}

impl Launcher {
    /// Creates the launcher's id. The launcher carries no placement: the slot
    /// holding it records where it sits.
    pub(crate) fn new(name: String, mode: LauncherMode, params: Params) -> Self {
        Self {
            id: LaunchProfileId::new(),
            name,
            mode,
            params,
            full_screen_mode: FullScreenMode::default(),
        }
    }

    /// The launcher's profile as carried by `AddLauncher` changes.
    pub fn profile(&self) -> LaunchProfile {
        LaunchProfile {
            name: self.name.clone(),
            mode: self.mode,
            params: self.params.clone(),
            full_screen_mode: self.full_screen_mode,
        }
    }

    /// Sets the Full Screen Mode the launcher's base instances follow
    /// (ADR 0014). Returns `Self` for parse-time chaining.
    pub(crate) fn with_full_screen_mode(mut self, full_screen_mode: FullScreenMode) -> Self {
        self.full_screen_mode = full_screen_mode;
        self
    }
}

/// The spawnable profile of a launcher, carried by changes and shown by the
/// `LauncherPresenter`.
#[derive(Debug, Clone)]
pub struct LaunchProfile {
    pub name: String,
    pub mode: LauncherMode,
    pub params: Params,
    pub full_screen_mode: FullScreenMode,
}

/// What a slot of a project's matrix hosts: a launcher or a nested project,
/// never both. A placement that hosts neither is simply empty, so a slot exists
/// only while it has content. The launcher record rides inside the variant —
/// there is no second parallel field the two could disagree in.
#[derive(Debug, Clone)]
pub enum SlotPayload {
    Launcher(Launcher),
    Project(ProjectId),
}

impl SlotPayload {}

/// The `Copy` id-level view of a [`SlotPayload`], for readers that only match on
/// which kind of content a slot holds.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From)]
pub enum SlotContent {
    Launcher(LaunchProfileId),
    Project(ProjectId),
}

impl SlotContent {
    /// The topology target that presents this content, mirroring
    /// [`SlotPayload::target`].
    pub fn target(self) -> DesktopTarget {
        match self {
            SlotContent::Launcher(launcher) => DesktopTarget::Launcher(launcher),
            SlotContent::Project(project) => DesktopTarget::Project(project),
        }
    }
}

/// The content an [`crate::desktop_system::ProjectCommand::AssignSlot`] assigns:
/// the content itself plus what is needed to create it.
///
/// A launcher carries its full profile, because assigning one creates it; a
/// project carries its name, because assigning a project name creates the nested
/// project and places it — there is no separate "add project" operation.
#[derive(Debug, Clone)]
pub enum SlotAssignment {
    Launcher {
        id: LaunchProfileId,
        profile: LaunchProfile,
    },
    Project {
        id: ProjectId,
        name: String,
    },
}

impl SlotAssignment {
    pub fn content(&self) -> SlotContent {
        match self {
            SlotAssignment::Launcher { id, .. } => SlotContent::Launcher(*id),
            SlotAssignment::Project { id, .. } => SlotContent::Project(*id),
        }
    }
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

    /// The root project's fixed id. It is a constant so the boot command can
    /// name the root before the system exists and the live model can never
    /// disagree with the aggregate about which project is the root.
    pub const ROOT: Self = Self(Uuid::nil());
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From, Into)]
pub struct LaunchProfileId(Uuid);

impl LaunchProfileId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifted_slots_moves_a_contiguous_run_in_reverse_order() {
        let configuration = configuration([
            ("first", MatrixPlacement { column: 1, row: 0 }),
            ("second", MatrixPlacement { column: 2, row: 0 }),
            ("third", MatrixPlacement { column: 3, row: 0 }),
        ]);

        let shifted = configuration
            .shifted_slots(
                root_id_of(&configuration),
                MatrixPlacement { column: 1, row: 0 },
                MoveDirection::Right,
            )
            .unwrap();

        assert_eq!(
            shifted,
            vec![
                (
                    MatrixPlacement { column: 3, row: 0 },
                    MatrixPlacement { column: 4, row: 0 }
                ),
                (
                    MatrixPlacement { column: 2, row: 0 },
                    MatrixPlacement { column: 3, row: 0 }
                ),
                (
                    MatrixPlacement { column: 1, row: 0 },
                    MatrixPlacement { column: 2, row: 0 }
                ),
            ]
        );
    }

    #[test]
    fn shifted_slots_stops_at_the_first_empty_slot() {
        let configuration = configuration([
            ("first", MatrixPlacement { column: 1, row: 0 }),
            ("second", MatrixPlacement { column: 3, row: 0 }),
        ]);

        let shifted = configuration
            .shifted_slots(
                root_id_of(&configuration),
                MatrixPlacement { column: 1, row: 0 },
                MoveDirection::Right,
            )
            .unwrap();

        assert_eq!(
            shifted,
            vec![(
                MatrixPlacement { column: 1, row: 0 },
                MatrixPlacement { column: 2, row: 0 }
            )]
        );
    }

    #[test]
    fn shifted_slots_rejects_left_and_up_boundaries() {
        let configuration = configuration([("only", MatrixPlacement { column: 0, row: 0 })]);

        assert!(
            configuration
                .shifted_slots(
                    root_id_of(&configuration),
                    MatrixPlacement { column: 0, row: 0 },
                    MoveDirection::Left
                )
                .is_err()
        );
        assert!(
            configuration
                .shifted_slots(
                    root_id_of(&configuration),
                    MatrixPlacement { column: 0, row: 0 },
                    MoveDirection::Up
                )
                .is_err()
        );
    }

    #[test]
    fn shifted_left_slots_pulls_the_same_row_left() {
        let configuration = configuration([
            ("a", MatrixPlacement { column: 0, row: 0 }),
            ("b", MatrixPlacement { column: 1, row: 0 }),
            ("c", MatrixPlacement { column: 2, row: 0 }),
            ("other-row", MatrixPlacement { column: 0, row: 1 }),
        ]);

        assert_eq!(
            configuration.shifted_left_slots(
                root_id_of(&configuration),
                MatrixPlacement { column: 0, row: 0 }
            ),
            vec![
                (
                    MatrixPlacement { column: 1, row: 0 },
                    MatrixPlacement { column: 0, row: 0 }
                ),
                (
                    MatrixPlacement { column: 2, row: 0 },
                    MatrixPlacement { column: 1, row: 0 }
                ),
            ]
        );
    }

    #[test]
    fn content_at_classifies_both_kinds_of_slot_content() {
        let nested = ProjectId::new();
        let placement = MatrixPlacement { column: 0, row: 0 };
        let mut configuration = configuration([("only", placement)]);
        let root = root_id_of(&configuration);

        let existing = configuration.launcher_at(root, placement).unwrap();
        assert_eq!(
            configuration.content_at(root, placement),
            Some(SlotContent::Launcher(existing))
        );

        configuration.assign_slot(Some(root), placement, nested_assignment(nested));
        assert_eq!(
            configuration.content_at(root, placement),
            Some(SlotContent::Project(nested))
        );
    }

    #[test]
    fn a_move_into_the_moved_subtree_is_rejected() {
        let placement = MatrixPlacement { column: 0, row: 0 };
        let nested = ProjectId::new();
        let mut configuration = configuration([("only", placement)]);
        let root = root_id_of(&configuration);
        configuration.assign_slot(Some(root), placement, named_project(nested, "nested"));

        let inner = MatrixPlacement { column: 0, row: 0 };
        assert!(!configuration.can_move_slot((root, placement), (nested, inner)));
        assert!(!configuration.move_slot((root, placement), (nested, inner)));
    }

    #[test]
    fn assigning_a_project_creates_it_in_the_parent_slot() {
        let placement = MatrixPlacement { column: 1, row: 0 };
        let nested = ProjectId::new();
        let mut configuration = configuration([("only", MatrixPlacement { column: 0, row: 0 })]);
        let root = root_id_of(&configuration);

        configuration.assign_slot(Some(root), placement, named_project(nested, "labs"));

        assert_eq!(
            configuration.child_project_at(root, placement),
            Some(nested)
        );
        assert_eq!(
            configuration
                .project(nested)
                .map(|project| project.name.as_str()),
            Some("labs")
        );
        assert_eq!(
            configuration
                .project(root)
                .and_then(|project| { project.placement_of_content(SlotContent::Project(nested)) }),
            Some(placement)
        );
    }

    #[test]
    fn clearing_a_slot_leaves_the_nested_project_empty_rather_than_removed() {
        let placement = MatrixPlacement { column: 0, row: 0 };
        let mut configuration = configuration([("only", placement)]);
        let root = root_id_of(&configuration);
        configuration.assign_slot(
            Some(root),
            placement,
            named_project(ProjectId::new(), "labs"),
        );

        configuration.clear_slot(root, placement);

        assert_eq!(configuration.content_at(root, placement), None);
        assert_eq!(
            configuration.projects().count(),
            2,
            "the nested project stays"
        );
    }

    #[test]
    fn removing_a_project_takes_its_subtree_and_frees_the_parent_slot() {
        let root_placement = MatrixPlacement { column: 0, row: 0 };
        let mut configuration = configuration([("only", MatrixPlacement { column: 1, row: 0 })]);
        let root = root_id_of(&configuration);
        let labs = ProjectId::new();
        configuration.assign_slot(Some(root), root_placement, named_project(labs, "labs"));
        let inner = ProjectId::new();
        configuration.assign_slot(
            Some(labs),
            MatrixPlacement { column: 0, row: 0 },
            named_project(inner, "inner"),
        );

        configuration.remove_project(labs);

        assert!(configuration.project(labs).is_none());
        assert!(configuration.project(inner).is_none());
        assert_eq!(configuration.content_at(root, root_placement), None);
    }

    #[test]
    fn launcher_count_and_first_launcher_walk_nested_projects() {
        let placement = MatrixPlacement { column: 0, row: 0 };
        let mut configuration = configuration([("top", MatrixPlacement { column: 0, row: 1 })]);
        let root = root_id_of(&configuration);
        let nested = ProjectId::new();
        configuration.assign_slot(Some(root), placement, named_project(nested, "labs"));
        configuration.assign_slot(
            Some(nested),
            placement,
            launcher_assignment_for(LaunchProfileId::new()),
        );

        assert_eq!(configuration.launcher_count(), 2);
        assert_eq!(
            configuration.first_launcher_depth_first(),
            configuration.launcher_at(root, MatrixPlacement { column: 0, row: 1 })
        );
    }

    #[test]
    fn first_launcher_descends_when_the_root_has_none_of_its_own() {
        let placement = MatrixPlacement { column: 0, row: 0 };
        let mut configuration = configuration([("top", MatrixPlacement { column: 0, row: 1 })]);
        let root = root_id_of(&configuration);
        configuration.clear_slot(root, MatrixPlacement { column: 0, row: 1 });
        let nested = ProjectId::new();
        configuration.assign_slot(Some(root), placement, named_project(nested, "labs"));
        let inner = LaunchProfileId::new();
        configuration.assign_slot(Some(nested), placement, launcher_assignment_for(inner));

        assert_eq!(configuration.first_launcher_depth_first(), Some(inner));
    }

    #[test]
    fn a_startup_address_path_resolves_relative_to_its_base() {
        let mut configuration =
            configuration([("root-launcher", MatrixPlacement { column: 0, row: 0 })]);
        let root = root_id_of(&configuration);
        let labs = ProjectId::new();
        configuration.assign_slot(
            Some(root),
            MatrixPlacement { column: 0, row: 1 },
            named_project(labs, "labs"),
        );
        let shell = LaunchProfileId::new();
        configuration.assign_slot(
            Some(labs),
            MatrixPlacement { column: 0, row: 0 },
            launcher_assignment_for(shell),
        );
        configuration
            .project_mut(labs)
            .expect("the launcher was just assigned")
            .launcher_mut(shell)
            .expect("the launcher was just assigned")
            .name = "shell".into();

        assert_eq!(
            configuration.resolve_launcher_path(root, "labs/shell"),
            Some(shell)
        );
        assert_eq!(
            configuration.resolve_launcher_path(labs, "shell"),
            Some(shell)
        );
        assert_eq!(
            configuration.resolve_launcher_path(root, "/labs/shell"),
            Some(shell)
        );
        assert_eq!(
            configuration
                .launcher_address_path(shell)
                .expect("the nested launcher has a representable root path"),
            "/labs/shell"
        );
        assert_eq!(configuration.resolve_project_path(root, "labs"), Some(labs));
        assert_eq!(
            configuration.resolve_launcher_path(root, "labs/missing"),
            None
        );
    }

    /// The boot command list opens with the root project's parentless assignment
    /// so its slots' parents exist before the slots are assigned under them.
    #[test]
    fn to_commands_starts_with_the_root_parentless_assign_slot() {
        let configuration = configuration([("shell", MatrixPlacement { column: 0, row: 0 })]);

        let commands = to_commands(&configuration);

        let mut commands = commands.into_iter();
        assert!(matches!(
            commands.next(),
            Some(ProjectCommand::SetStartupPath(None))
        ));
        assert!(matches!(
            commands.next(),
            Some(ProjectCommand::AssignSlot {
                parent: None,
                content: SlotAssignment::Project {
                    id: ProjectId::ROOT,
                    ..
                },
                ..
            })
        ));
        assert!(matches!(
            commands.next(),
            Some(ProjectCommand::AssignSlot {
                parent: Some(ProjectId::ROOT),
                ..
            })
        ));
    }

    /// A root project holding the given launchers, with the parent links derived
    /// the way parsing derives them.
    fn configuration(
        entries: impl IntoIterator<Item = (&'static str, MatrixPlacement)>,
    ) -> RuntimeConfiguration {
        let slots = entries
            .into_iter()
            .map(|(name, placement)| Slot::launcher(placement, launcher(name)))
            .collect();
        let mut root = Project::new(ROOT_PROJECT_NAME.into(), slots);
        root.id = ProjectId::ROOT;
        RuntimeConfiguration {
            projects: IndexMap::from([(root.id, root)]),
            startup: None,
            startup_path: None,
        }
    }

    fn nested_assignment(id: ProjectId) -> SlotAssignment {
        named_project(id, "labs")
    }

    fn launcher(name: &str) -> Launcher {
        Launcher::new(name.into(), LauncherMode::Visor, Params::new())
    }

    fn named_project(id: ProjectId, name: &str) -> SlotAssignment {
        SlotAssignment::Project {
            id,
            name: name.into(),
        }
    }

    fn launcher_assignment_for(id: LaunchProfileId) -> SlotAssignment {
        SlotAssignment::Launcher {
            id,
            profile: LaunchProfile {
                name: format!("{id:?}"),
                mode: LauncherMode::Visor,
                params: Params::new(),
                full_screen_mode: FullScreenMode::Regular,
            },
        }
    }

    fn root_id_of(_configuration: &RuntimeConfiguration) -> ProjectId {
        ProjectId::ROOT
    }
}
