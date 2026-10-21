//! The desktop configuration aggregate: projects whose matrices hold slots, each
//! slot a launcher or a nested project, and the launcher the session boots into.
//!
//! Projects are stored flat and nest by id link (ADR 0011): a project's slots
//! name the launchers and projects they host, so no recursive type is needed.

use std::collections::HashSet;
use std::ops::Index;

use anyhow::{Context, Result, bail, ensure};
use derive_more::{From, Into};
use euclid::vec2;
use indexmap::IndexMap;
use log::warn;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use massive_applications::MoveDirection;
use massive_util::CollectingVec;

use crate::desktop_system::{ConfigurationCommand, DesktopTarget, FocusableTarget, SlotShift};

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

/// The Full Screen Mode shared by a launcher's primary instances (ADR 0014).
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

    /// The nested project in `project`'s `slot`, if any.
    #[cfg(test)]
    pub fn child_project_at(&self, project: ProjectId, slot: SlotPosition) -> Option<ProjectId> {
        match self.project(project)?.content_at(slot)? {
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
    // construction path (`new`), where positions come from the document and a
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

    /// Assigns `content` to `parent`'s slot at `position`, returning the host
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
        position: SlotPosition,
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
                project.assign_slot(Slot::launcher(position, launcher));
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
                        .assign_slot(Slot::project(position, id));
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

    /// Unassigns `project`'s `slot`. An unassigned slot is not stored, so clearing
    /// it again does nothing.
    pub fn clear_slot(&mut self, project: ProjectId, slot: SlotPosition) {
        self.project_mut(project)
            .expect("the project hosting the slot exists")
            .remove_slot(slot);
    }

    /// Moves the content of `source` to `dest`, returning whether the move
    /// happened.
    ///
    /// Rejected — with no change emitted — when the source slot is empty, when the
    /// destination is inside the moved project's own subtree, or when the
    /// destination project is not in the tree.
    pub fn move_slot(
        &mut self,
        source: (ProjectId, SlotPosition),
        dest: (ProjectId, SlotPosition),
    ) -> bool {
        let (source_project, source_position) = source;
        let (dest_project, dest_position) = dest;

        if source == dest || self.project(dest_project).is_none() {
            return false;
        }
        let Some(content) = self.content_at(source_project, source_position) else {
            return false;
        };
        if let SlotContent::Project(moved) = content
            && self.is_in_subtree(moved, dest_project)
        {
            return false;
        }

        let Some(index) = self
            .project(source_project)
            .and_then(|project| project.slot_index(source_position))
        else {
            return false;
        };
        let Some(source_project_mut) = self.project_mut(source_project) else {
            return false;
        };
        let mut slot = source_project_mut.slots.remove(index);
        // The destination and the removed slot are distinct positions, so
        // re-adding under the new position cannot collide with the source slot.
        slot.position = dest_position;
        self.project_mut(dest_project)
            .expect("the destination project was checked above")
            .assign_slot(slot);
        true
    }

    /// Wraps `parent`'s `grouped` slots into a new project with the id `project`. The slots move
    /// into the new project and the project takes their place in `parent`, both where
    /// [`group_landings`] says.
    ///
    /// This is one step, because the project's slot is usually one of the grouped slots.
    /// With separate moves and assignments, one of them would first have to wait in a temporary
    /// slot.
    pub fn group_slots(
        &mut self,
        parent: ProjectId,
        grouped: &[SlotPosition],
        project: ProjectId,
        name: &str,
    ) {
        let (project_slot, landings) = group_landings(grouped);
        let parent_project = self
            .project_mut(parent)
            .expect("the parent of grouped slots exists");
        let slots: Vec<Slot> = landings
            .into_iter()
            .map(|(outer, inner)| {
                let index = parent_project
                    .slot_index(outer)
                    .expect("a grouped slot has content");
                let mut slot = parent_project.slots.remove(index);
                slot.position = inner;
                slot
            })
            .collect();

        let name = self.new_project_name(project, DEFAULT_NEW_PROJECT_NAME, name);
        self.add_project(project, name);
        let grouped = self
            .project_mut(project)
            .expect("the new project was just added");
        for slot in slots {
            grouped.assign_slot(slot);
        }
        self.project_mut(parent)
            .expect("the parent of grouped slots exists")
            .assign_slot(Slot::project(project_slot, project));
    }

    /// Moves the slots of the project in `parent`'s `grouped` slot into `parent` and removes the
    /// project. Each slot lands where [`Self::ungroup_landings`] says. The landing slots must be
    /// unassigned, which the planned collision pushes ([`Self::ungroup_pushes`]) make sure of.
    ///
    /// This is one step for the same reason as [`Self::group_slots`]. The project's first slot
    /// usually lands on the project's own slot, so the project has to leave it at the same time.
    ///
    /// Returns the id of the removed project, whose presenter the caller removes.
    pub fn ungroup_slot(&mut self, parent: ProjectId, grouped: SlotPosition) -> ProjectId {
        let Some(SlotContent::Project(project)) = self.content_at(parent, grouped) else {
            panic!("an ungrouped slot holds a project");
        };
        let landings = self.ungroup_landings(project, grouped);
        let grouped_project = self
            .projects
            .shift_remove(&project)
            .expect("the ungrouped project exists");

        let parent_project = self
            .project_mut(parent)
            .expect("the parent of an ungrouped slot exists");
        parent_project.remove_slot(grouped);
        for (mut slot, (_, landing)) in grouped_project.slots.into_iter().zip(landings) {
            assert!(
                parent_project.content_at(landing).is_none(),
                "the landing slot {landing:?} was cleared before ungrouping"
            );
            slot.position = landing;
            parent_project.assign_slot(slot);
        }
        project
    }

    /// Where each slot of `project` lands in its parent when the project in the `grouped` slot is
    /// ungrouped, as pairs of the slot inside the project and its landing slot. The slots keep
    /// their arrangement. The top-left corner of their bounding box lands on `grouped`, so a
    /// project that grouping created gives back exactly what was grouped.
    ///
    /// The pairs are in the project's slot order.
    pub fn ungroup_landings(
        &self,
        project: ProjectId,
        grouped: SlotPosition,
    ) -> Vec<(SlotPosition, SlotPosition)> {
        let inner: Vec<SlotPosition> = self
            .project(project)
            .expect("the ungrouped project exists")
            .slot_contents()
            .map(|(inner, _)| inner)
            .collect();
        let min_column = inner.iter().map(|inner| inner.column).min();
        let min_row = inner.iter().map(|inner| inner.row).min();
        let (Some(min_column), Some(min_row)) = (min_column, min_row) else {
            return Vec::new();
        };
        inner
            .into_iter()
            .map(|inner| {
                let landing = SlotPosition {
                    column: grouped.column + inner.column - min_column,
                    row: grouped.row + inner.row - min_row,
                };
                (inner, landing)
            })
            .collect()
    }

    /// The moves in `parent` that clear the `landings` of an ungrouping, before the slots of the
    /// project in the `grouped` slot land there. The project's own slot counts as unassigned,
    /// because the project leaves it.
    ///
    /// Only rows in which a landing slot is assigned are touched. In such a row, the slots from
    /// the first assigned landing slot onwards are pushed right, so that the first of them ends up
    /// just after the row's last landing slot. Each further slot moves only as far as
    /// needed to stay right of the slot before it. The push therefore stops at the first slot that
    /// already sits right of the slot before it. Unassigned slots between two slots absorb as many
    /// columns of the push as there are of them.
    ///
    /// The moves are ordered so that each one goes to an unassigned slot.
    pub fn ungroup_pushes(
        &self,
        parent: ProjectId,
        grouped: SlotPosition,
        landings: &[SlotPosition],
    ) -> Vec<(SlotPosition, SlotPosition)> {
        let parent_project = self
            .project(parent)
            .expect("the parent of an ungrouped slot exists");
        let is_taken =
            |slot: SlotPosition| slot != grouped && parent_project.content_at(slot).is_some();

        let mut rows: Vec<u32> = landings.iter().map(|landing| landing.row).collect();
        rows.sort_unstable();
        rows.dedup();

        let mut pushes = Vec::new();
        for row in rows {
            let columns = landings
                .iter()
                .filter(|landing| landing.row == row)
                .map(|landing| landing.column);
            let Some(first_taken) = columns
                .clone()
                .filter(|&column| is_taken(SlotPosition { column, row }))
                .min()
            else {
                continue;
            };
            let after_landings = columns.max().expect("a landing row has landings") + 1;

            let mut row_pushes = Vec::new();
            let mut next_free = after_landings;
            for (from, _) in parent_project.slot_contents() {
                if from.row != row || from.column < first_taken || from == grouped {
                    continue;
                }
                let column = from.column.max(next_free);
                if column == from.column {
                    break;
                }
                row_pushes.push((from, SlotPosition { column, row }));
                next_free = column + 1;
            }
            // Rightmost first, so every push lands on a slot that is already unassigned.
            pushes.extend(row_pushes.into_iter().rev());
        }
        pushes
    }

    /// Exchanges the contents of `project`'s slots at `a` and `b`. Both slots hold content.
    pub fn swap_slots(&mut self, project: ProjectId, a: SlotPosition, b: SlotPosition) {
        let project = self
            .project_mut(project)
            .expect("the project of swapped slots exists");
        let index_a = project.slot_index(a).expect("a swapped slot has content");
        let index_b = project.slot_index(b).expect("a swapped slot has content");
        project.slots[index_a].position = b;
        project.slots[index_b].position = a;
        // The aggregate's readers rely on slots in position order.
        sort_slots(&mut project.slots);
    }

    /// Whether the content of `source` can move to `dest`.
    ///
    /// Rejected when the source slot is empty, when the destination project is not
    /// in the tree, and when the destination is inside the moved project's own
    /// subtree — a project cannot be moved into itself.
    pub fn can_move_slot(
        &self,
        source: (ProjectId, SlotPosition),
        dest: (ProjectId, SlotPosition),
    ) -> bool {
        let (source_project, source_slot) = source;
        let (dest_project, _) = dest;

        if source == dest || self.project(dest_project).is_none() {
            return false;
        }
        let Some(content) = self.content_at(source_project, source_slot) else {
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
    // Slot lookup, assignment checks, and the two shift-sequence computations.

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
                project
                    .slot_contents()
                    .find_map(|(_, content)| match content {
                        SlotContent::Launcher(launcher) => Some(launcher),
                        SlotContent::Project(_) => None,
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

    /// The launcher assigned to `project`'s `slot`, if any. Slot positions are
    /// unique per project, so this is a lookup, not a search over ties.
    #[cfg(test)]
    pub fn launcher_at(&self, project: ProjectId, slot: SlotPosition) -> Option<LaunchProfileId> {
        self.project(project)
            .and_then(|project| project.launcher_at(slot))
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
    /// on screen: Project removal by name removes the nearest one.
    pub fn project_index(&self, project: ProjectId) -> Option<usize> {
        self.projects.get_index_of(&project)
    }

    /// The launchers of `project` that must move for the content in `slot`
    /// to step `direction`: the contiguous run of assigned slots ahead of it,
    /// listed so that applying the moves back-to-front stays conflict-free.
    ///
    /// The shift is content-agnostic per slot — a project-assigned slot shifts
    /// like a launcher-assigned one.
    pub fn shifted_slots(
        &self,
        project: ProjectId,
        slot: SlotPosition,
        direction: MoveDirection,
    ) -> Result<Vec<(SlotPosition, SlotPosition)>> {
        let mut run = vec![slot];
        loop {
            let leading = *run.last().expect("run is never empty");
            let Some(next) = leading.neighbor(direction) else {
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
                let next = moved
                    .neighbor(direction)
                    .expect("the run's slots were validated before shifting");
                (moved, next)
            })
            .collect())
    }

    /// The assigned slots right of `slot` in the same row, each moved one
    /// column left — the shift emitted when a slot is unassigned.
    pub fn shifted_left_slots(
        &self,
        project: ProjectId,
        slot: SlotPosition,
    ) -> Vec<(SlotPosition, SlotPosition)> {
        self.slots_ordered(project)
            .filter(|(candidate, _)| candidate.row == slot.row && candidate.column > slot.column)
            .map(|(candidate, _)| {
                (
                    candidate,
                    SlotPosition {
                        column: candidate.column - 1,
                        row: candidate.row,
                    },
                )
            })
            .collect()
    }

    /// The position index of the launcher within its project, counting the launchers in
    /// position order: Manhattan distance between those indexes is the matrix distance
    /// Launcher removal by name picks the nearest launcher by.
    pub fn launcher_index(&self, launcher: LaunchProfileId) -> Option<usize> {
        let project = self.project_of_launcher(launcher)?;
        project.launchers().position(|l| l.id == launcher)
    }

    /// The project and position of the slot holding `content`.
    pub fn slot_of_content(
        &self,
        content: impl Into<SlotContent>,
    ) -> Option<(ProjectId, SlotPosition)> {
        let content = content.into();
        self.projects.values().find_map(|project| {
            project
                .position_of_content(content)
                .map(|position| (project.id, position))
        })
    }

    /// The content of `project`'s `slot`, `None` when the slot is unassigned.
    /// This is the assignment query the shift planner and `AssignSlot`
    /// validation need; slot positions are unique per project, so it is a lookup.
    pub fn content_at(&self, project: ProjectId, slot: SlotPosition) -> Option<SlotContent> {
        self.project(project)
            .and_then(|project| project.content_at(slot))
    }

    /// All launchers of the project, kept in matrix-position order (the
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

    /// Every assigned slot of `project`, in position order. `SlotPosition`
    /// and `SlotContent` are both `Copy`, so the items are values without a
    /// collection to build first.
    pub fn slots_ordered(
        &self,
        project: ProjectId,
    ) -> impl Iterator<Item = (SlotPosition, SlotContent)> + '_ {
        self.project(project)
            .into_iter()
            .flat_map(|project| project.slot_contents())
    }

    /// The first assigned slot of `project` in matrix-position order.
    pub fn first_slot(&self, project: ProjectId) -> Option<(SlotPosition, SlotContent)> {
        self.slots_ordered(project).next()
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
pub fn to_commands(configuration: &RuntimeConfiguration) -> CollectingVec<ConfigurationCommand> {
    let mut commands = CollectingVec::Empty;

    commands.push(ConfigurationCommand::SetStartupPath(
        configuration.startup_path.clone(),
    ));
    let root_name = configuration
        .project(ProjectId::ROOT)
        .map(|project| project.name.clone())
        .unwrap_or_else(|| ROOT_PROJECT_NAME.to_string());
    commands.push(ConfigurationCommand::AssignSlot {
        parent: None,
        slot: SlotPosition { column: 0, row: 0 },
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
    commands: &mut CollectingVec<ConfigurationCommand>,
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

        commands.push(ConfigurationCommand::AssignSlot {
            parent: Some(project),
            slot: slot.position,
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
/// The vector is kept sorted by position (`sort_slots`): the shift computations
/// and nearest-by-name picks read slots in position order, so readers can rely
/// on order without re-sorting.
#[derive(Debug, Clone)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    slots: Vec<Slot>,
}

impl Project {
    /// Creates the project's id and puts its slots into position order.
    ///
    /// The sort is stable, so slots sharing a position keep their document order.
    pub(crate) fn new(name: String, mut slots: Vec<Slot>) -> Self {
        sort_slots(&mut slots);
        Self {
            id: ProjectId::new(),
            name,
            slots,
        }
    }

    /// Puts `slot` into the matrix, replacing whatever content is assigned to
    /// its position.
    ///
    /// An assign into an assigned slot cannot displace the assigned content —
    /// the shift happens in the plan, before the live model sees the assignment.
    pub(crate) fn assign_slot(&mut self, slot: Slot) {
        if let Some(index) = self.slot_index(slot.position) {
            self.slots[index] = slot;
            return;
        }
        self.slots.push(slot);
        sort_slots(&mut self.slots);
    }

    /// The launcher assigned to `slot`, if it holds one.
    #[cfg(test)]
    pub fn launcher_at(&self, slot: SlotPosition) -> Option<LaunchProfileId> {
        match self.content_at(slot)? {
            SlotContent::Launcher(launcher) => Some(launcher),
            SlotContent::Project(_) => None,
        }
    }

    /// The content of `slot`, `None` when it is unassigned.
    pub fn content_at(&self, slot: SlotPosition) -> Option<SlotContent> {
        self.slot_index(slot)
            .map(|index| self.slots[index].content())
    }

    /// The position and content id of every slot, in position order. This is the view for code
    /// that only needs to know what sits where.
    pub fn slot_contents(&self) -> impl Iterator<Item = (SlotPosition, SlotContent)> + '_ {
        self.slots
            .iter()
            .map(|slot| (slot.position, slot.content()))
    }

    /// The slots with their full payload, including each launcher's record. Only code that needs
    /// those records uses this, such as persistence and the boot replay. Everything else uses
    /// [`Self::slot_contents`].
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

    /// The launchers of this project's matrix, in position order.
    pub fn launchers(&self) -> impl Iterator<Item = &Launcher> {
        self.slots.iter().filter_map(|slot| match &slot.content {
            SlotPayload::Launcher(launcher) => Some(launcher),
            SlotPayload::Project(_) => None,
        })
    }

    /// The ids of the projects nested in this project's matrix, in position order.
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

    /// The first project nested in this matrix, in position order — the walk
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

    /// The position of the slot whose ids match `content`.
    pub fn position_of_content(&self, content: impl Into<SlotContent>) -> Option<SlotPosition> {
        let content = content.into();
        self.slots
            .iter()
            .find(|slot| slot.content() == content)
            .map(|slot| slot.position)
    }

    /// The launcher called `name` of this matrix, nearest first in position
    /// order — duplicates address the first.
    pub fn launcher_by_name(&self, name: &str) -> Option<LaunchProfileId> {
        self.slots.iter().find_map(|slot| match &slot.content {
            SlotPayload::Launcher(launcher) if launcher.name == name => Some(launcher.id),
            _ => None,
        })
    }

    /// Unassigns `slot`, if it is assigned.
    pub(crate) fn remove_slot(&mut self, slot: SlotPosition) {
        self.slots.retain(|stored| stored.position != slot);
    }

    /// The index of `slot` among the stored slots, if it is assigned.
    pub fn slot_index(&self, slot: SlotPosition) -> Option<usize> {
        self.slots.iter().position(|stored| stored.position == slot)
    }
}

/// Puts slots into position order: row first, then column, matching the order the
/// aggregate's readers rely on.
fn sort_slots(slots: &mut [Slot]) {
    slots.sort_by_key(|slot| (slot.position.row, slot.position.column));
}

/// One assigned slot of a project's matrix.
///
/// Only assigned slots are stored (ADR 0011). A slot is addressed by its
/// `(project, position)` key rather than by an id. The
/// position is `Slot`'s own field — the launcher payload carries none — so a
/// moved slot cannot diverge from its content.
#[derive(Debug, Clone)]
pub struct Slot {
    pub position: SlotPosition,
    pub content: SlotPayload,
}

impl Slot {
    /// Builds a launcher-assigned slot, which is how slots are created at parse
    /// and on assign.
    pub(crate) fn launcher(position: SlotPosition, launcher: Launcher) -> Self {
        Self {
            position,
            content: SlotPayload::Launcher(launcher),
        }
    }

    /// Builds a project-assigned slot.
    pub(crate) fn project(position: SlotPosition, project: ProjectId) -> Self {
        Self {
            position,
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

/// The aggregate keeps a project's slots sorted by position, so readers index by
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
    /// The Full Screen Mode all of the launcher's primary instances follow
    /// (ADR 0014). Persisted with the configuration.
    pub full_screen_mode: FullScreenMode,
}

impl Launcher {
    /// Creates the launcher's id. The launcher carries no position: the slot
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

    /// Sets the Full Screen Mode the launcher's primary instances follow
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
/// never both. A slot that hosts neither is unassigned and is not stored. The launcher record rides inside the variant —
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
            SlotContent::Project(project) => project.into(),
        }
    }

    /// The keyboard-focus target for this content: a project is focused as a slot.
    pub fn focusable_target(self) -> FocusableTarget {
        match self {
            SlotContent::Launcher(launcher) => FocusableTarget::Launcher(launcher),
            SlotContent::Project(project) => FocusableTarget::Project(project),
        }
    }
}

/// The content an [`crate::desktop_system::ConfigurationCommand::AssignSlot`] assigns:
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

/// A slot's column and row in its project's matrix. The slot there may be assigned or unassigned.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SlotPosition {
    pub column: u32,
    pub row: u32,
}

/// Coordinates in a project's matrix, counted in columns along x and rows along y (ADR 0028).
pub struct MatrixUnit;

/// A position as a point in matrix coordinates, the column along x and the row along y.
pub type MatrixPoint = euclid::Point2D<u32, MatrixUnit>;

/// A rectangle of positions in a matrix. Like every `Box2D`, it excludes its maximum, so `max` lies
/// one column and one row past the bottom-right position it contains.
pub type MatrixRect = euclid::Box2D<u32, MatrixUnit>;

/// The smallest rectangle of positions that contains all `points`, of which there is at least
/// one.
pub fn matrix_rect_covering(points: impl IntoIterator<Item = MatrixPoint>) -> MatrixRect {
    let rect = MatrixRect::from_points(points);
    MatrixRect::new(rect.min, rect.max + vec2(1, 1))
}

/// Where grouping the `grouped` slots puts them (ADR 0026). The new project takes the top-left
/// slot of their bounding box, and each slot keeps its offset from that corner inside the project.
/// Returns the project's slot and, for each grouped slot, the slot inside the project it moves to.
/// There is at least one grouped slot.
pub fn group_landings(
    grouped: &[SlotPosition],
) -> (SlotPosition, Vec<(SlotPosition, SlotPosition)>) {
    let top_left = matrix_rect_covering(grouped.iter().map(|&slot| slot.into())).min;
    let landings = grouped
        .iter()
        .map(|&slot| {
            let inner = MatrixPoint::from(slot) - top_left.to_vector();
            (slot, inner.into())
        })
        .collect();
    (top_left.into(), landings)
}

impl From<SlotPosition> for MatrixPoint {
    fn from(position: SlotPosition) -> Self {
        Self::new(position.column, position.row)
    }
}

impl From<MatrixPoint> for SlotPosition {
    fn from(point: MatrixPoint) -> Self {
        Self {
            column: point.x,
            row: point.y,
        }
    }
}

impl SlotPosition {
    /// The column and the row, indexed by axis: the column along the horizontal axis 0, the row
    /// along the vertical axis 1.
    pub fn indices(self) -> [usize; 2] {
        [self.column as usize, self.row as usize]
    }

    /// The neighboring slot one `direction` step away, `None` past the matrix edge.
    pub fn neighbor(self, direction: MoveDirection) -> Option<SlotPosition> {
        match direction {
            MoveDirection::Left => self.column.checked_sub(1).map(|column| SlotPosition {
                column,
                row: self.row,
            }),
            MoveDirection::Right => self.column.checked_add(1).map(|column| SlotPosition {
                column,
                row: self.row,
            }),
            MoveDirection::Up => self.row.checked_sub(1).map(|row| SlotPosition {
                row,
                column: self.column,
            }),
            MoveDirection::Down => self.row.checked_add(1).map(|row| SlotPosition {
                row,
                column: self.column,
            }),
        }
    }
}

impl From<(u32, u32)> for SlotPosition {
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

    /// Ungrouping pushes the slots in its way only as far as needed. Here `y` has to move two
    /// columns. With one unassigned slot behind it, `w` has to move one column too. With two
    /// unassigned slots behind it, `w` stays.
    #[test]
    fn ungroup_pushes_move_blocking_slots_just_far_enough() {
        let at = |column| SlotPosition { column, row: 0 };

        let one_unassigned_slot = configuration([("p", at(0)), ("y", at(1)), ("w", at(3))]);
        assert_eq!(
            one_unassigned_slot.ungroup_pushes(
                root_id_of(&one_unassigned_slot),
                at(0),
                &[at(0), at(1), at(2)]
            ),
            vec![(at(3), at(4)), (at(1), at(3))]
        );

        let two_unassigned_slots = configuration([("p", at(0)), ("y", at(1)), ("w", at(4))]);
        assert_eq!(
            two_unassigned_slots.ungroup_pushes(
                root_id_of(&two_unassigned_slots),
                at(0),
                &[at(0), at(1), at(2)]
            ),
            vec![(at(1), at(3))]
        );
    }

    /// A swap exchanges the two contents and leaves the slots in position order, the order the
    /// aggregate's readers rely on.
    #[test]
    fn swap_slots_exchanges_contents_and_keeps_position_order() {
        let first_at = SlotPosition { column: 0, row: 0 };
        let second_at = SlotPosition { column: 1, row: 1 };
        let mut configuration = configuration([("first", first_at), ("second", second_at)]);
        let root = root_id_of(&configuration);
        let id_of = |configuration: &RuntimeConfiguration, name| {
            SlotContent::Launcher(configuration[root].launcher_by_name(name).unwrap())
        };
        let (first, second) = (
            id_of(&configuration, "first"),
            id_of(&configuration, "second"),
        );

        configuration.swap_slots(root, first_at, second_at);

        let project = &configuration[root];
        assert_eq!(project.content_at(first_at), Some(second));
        assert_eq!(project.content_at(second_at), Some(first));
        let positions: Vec<_> = project
            .slot_contents()
            .map(|(position, _)| position)
            .collect();
        assert_eq!(positions, [first_at, second_at]);
    }

    #[test]
    fn shifted_slots_moves_a_contiguous_run_in_reverse_order() {
        let configuration = configuration([
            ("first", SlotPosition { column: 1, row: 0 }),
            ("second", SlotPosition { column: 2, row: 0 }),
            ("third", SlotPosition { column: 3, row: 0 }),
        ]);

        let shifted = configuration
            .shifted_slots(
                root_id_of(&configuration),
                SlotPosition { column: 1, row: 0 },
                MoveDirection::Right,
            )
            .unwrap();

        assert_eq!(
            shifted,
            vec![
                (
                    SlotPosition { column: 3, row: 0 },
                    SlotPosition { column: 4, row: 0 }
                ),
                (
                    SlotPosition { column: 2, row: 0 },
                    SlotPosition { column: 3, row: 0 }
                ),
                (
                    SlotPosition { column: 1, row: 0 },
                    SlotPosition { column: 2, row: 0 }
                ),
            ]
        );
    }

    #[test]
    fn shifted_slots_stops_at_the_first_empty_slot() {
        let configuration = configuration([
            ("first", SlotPosition { column: 1, row: 0 }),
            ("second", SlotPosition { column: 3, row: 0 }),
        ]);

        let shifted = configuration
            .shifted_slots(
                root_id_of(&configuration),
                SlotPosition { column: 1, row: 0 },
                MoveDirection::Right,
            )
            .unwrap();

        assert_eq!(
            shifted,
            vec![(
                SlotPosition { column: 1, row: 0 },
                SlotPosition { column: 2, row: 0 }
            )]
        );
    }

    #[test]
    fn shifted_slots_rejects_left_and_up_boundaries() {
        let configuration = configuration([("only", SlotPosition { column: 0, row: 0 })]);

        assert!(
            configuration
                .shifted_slots(
                    root_id_of(&configuration),
                    SlotPosition { column: 0, row: 0 },
                    MoveDirection::Left
                )
                .is_err()
        );
        assert!(
            configuration
                .shifted_slots(
                    root_id_of(&configuration),
                    SlotPosition { column: 0, row: 0 },
                    MoveDirection::Up
                )
                .is_err()
        );
    }

    #[test]
    fn shifted_left_slots_pulls_the_same_row_left() {
        let configuration = configuration([
            ("a", SlotPosition { column: 0, row: 0 }),
            ("b", SlotPosition { column: 1, row: 0 }),
            ("c", SlotPosition { column: 2, row: 0 }),
            ("other-row", SlotPosition { column: 0, row: 1 }),
        ]);

        assert_eq!(
            configuration.shifted_left_slots(
                root_id_of(&configuration),
                SlotPosition { column: 0, row: 0 }
            ),
            vec![
                (
                    SlotPosition { column: 1, row: 0 },
                    SlotPosition { column: 0, row: 0 }
                ),
                (
                    SlotPosition { column: 2, row: 0 },
                    SlotPosition { column: 1, row: 0 }
                ),
            ]
        );
    }

    #[test]
    fn content_at_classifies_both_kinds_of_slot_content() {
        let nested = ProjectId::new();
        let position = SlotPosition { column: 0, row: 0 };
        let mut configuration = configuration([("only", position)]);
        let root = root_id_of(&configuration);

        let existing = configuration.launcher_at(root, position).unwrap();
        assert_eq!(
            configuration.content_at(root, position),
            Some(SlotContent::Launcher(existing))
        );

        configuration.assign_slot(Some(root), position, nested_assignment(nested));
        assert_eq!(
            configuration.content_at(root, position),
            Some(SlotContent::Project(nested))
        );
    }

    #[test]
    fn a_move_into_the_moved_subtree_is_rejected() {
        let position = SlotPosition { column: 0, row: 0 };
        let nested = ProjectId::new();
        let mut configuration = configuration([("only", position)]);
        let root = root_id_of(&configuration);
        configuration.assign_slot(Some(root), position, named_project(nested, "nested"));

        let inner = SlotPosition { column: 0, row: 0 };
        assert!(!configuration.can_move_slot((root, position), (nested, inner)));
        assert!(!configuration.move_slot((root, position), (nested, inner)));
    }

    #[test]
    fn assigning_a_project_creates_it_in_the_parent_slot() {
        let position = SlotPosition { column: 1, row: 0 };
        let nested = ProjectId::new();
        let mut configuration = configuration([("only", SlotPosition { column: 0, row: 0 })]);
        let root = root_id_of(&configuration);

        configuration.assign_slot(Some(root), position, named_project(nested, "labs"));

        assert_eq!(configuration.child_project_at(root, position), Some(nested));
        assert_eq!(
            configuration
                .project(nested)
                .map(|project| project.name.as_str()),
            Some("labs")
        );
        assert_eq!(
            configuration
                .project(root)
                .and_then(|project| { project.position_of_content(SlotContent::Project(nested)) }),
            Some(position)
        );
    }

    #[test]
    fn clearing_a_slot_leaves_the_nested_project_empty_rather_than_removed() {
        let position = SlotPosition { column: 0, row: 0 };
        let mut configuration = configuration([("only", position)]);
        let root = root_id_of(&configuration);
        configuration.assign_slot(
            Some(root),
            position,
            named_project(ProjectId::new(), "labs"),
        );

        configuration.clear_slot(root, position);

        assert_eq!(configuration.content_at(root, position), None);
        assert_eq!(
            configuration.projects().count(),
            2,
            "the nested project stays"
        );
    }

    #[test]
    fn removing_a_project_takes_its_subtree_and_frees_the_parent_slot() {
        let root_position = SlotPosition { column: 0, row: 0 };
        let mut configuration = configuration([("only", SlotPosition { column: 1, row: 0 })]);
        let root = root_id_of(&configuration);
        let labs = ProjectId::new();
        configuration.assign_slot(Some(root), root_position, named_project(labs, "labs"));
        let inner = ProjectId::new();
        configuration.assign_slot(
            Some(labs),
            SlotPosition { column: 0, row: 0 },
            named_project(inner, "inner"),
        );

        configuration.remove_project(labs);

        assert!(configuration.project(labs).is_none());
        assert!(configuration.project(inner).is_none());
        assert_eq!(configuration.content_at(root, root_position), None);
    }

    #[test]
    fn launcher_count_and_first_launcher_walk_nested_projects() {
        let position = SlotPosition { column: 0, row: 0 };
        let mut configuration = configuration([("top", SlotPosition { column: 0, row: 1 })]);
        let root = root_id_of(&configuration);
        let nested = ProjectId::new();
        configuration.assign_slot(Some(root), position, named_project(nested, "labs"));
        configuration.assign_slot(
            Some(nested),
            position,
            launcher_assignment_for(LaunchProfileId::new()),
        );

        assert_eq!(configuration.launcher_count(), 2);
        assert_eq!(
            configuration.first_launcher_depth_first(),
            configuration.launcher_at(root, SlotPosition { column: 0, row: 1 })
        );
    }

    #[test]
    fn first_launcher_descends_when_the_root_has_none_of_its_own() {
        let position = SlotPosition { column: 0, row: 0 };
        let mut configuration = configuration([("top", SlotPosition { column: 0, row: 1 })]);
        let root = root_id_of(&configuration);
        configuration.clear_slot(root, SlotPosition { column: 0, row: 1 });
        let nested = ProjectId::new();
        configuration.assign_slot(Some(root), position, named_project(nested, "labs"));
        let inner = LaunchProfileId::new();
        configuration.assign_slot(Some(nested), position, launcher_assignment_for(inner));

        assert_eq!(configuration.first_launcher_depth_first(), Some(inner));
    }

    #[test]
    fn a_startup_address_path_resolves_relative_to_its_base() {
        let mut configuration =
            configuration([("root-launcher", SlotPosition { column: 0, row: 0 })]);
        let root = root_id_of(&configuration);
        let labs = ProjectId::new();
        configuration.assign_slot(
            Some(root),
            SlotPosition { column: 0, row: 1 },
            named_project(labs, "labs"),
        );
        let shell = LaunchProfileId::new();
        configuration.assign_slot(
            Some(labs),
            SlotPosition { column: 0, row: 0 },
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
        let configuration = configuration([("shell", SlotPosition { column: 0, row: 0 })]);

        let commands = to_commands(&configuration);

        let mut commands = commands.into_iter();
        assert!(matches!(
            commands.next(),
            Some(ConfigurationCommand::SetStartupPath(None))
        ));
        assert!(matches!(
            commands.next(),
            Some(ConfigurationCommand::AssignSlot {
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
            Some(ConfigurationCommand::AssignSlot {
                parent: Some(ProjectId::ROOT),
                ..
            })
        ));
    }

    /// A root project holding the given launchers, with the parent links derived
    /// the way parsing derives them.
    fn configuration(
        entries: impl IntoIterator<Item = (&'static str, SlotPosition)>,
    ) -> RuntimeConfiguration {
        let slots = entries
            .into_iter()
            .map(|(name, position)| Slot::launcher(position, launcher(name)))
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

    /// The new project sits at the top-left corner of the grouped slots' bounding box, even when
    /// that corner is empty, and the slots keep their arrangement inside it.
    #[test]
    fn grouped_slots_keep_their_arrangement_from_their_top_left_corner() {
        let at = |column, row| SlotPosition { column, row };

        let (position, landings) = group_landings(&[at(2, 1), at(1, 2)]);

        assert_eq!(position, at(1, 1));
        assert_eq!(landings, [(at(2, 1), at(1, 0)), (at(1, 2), at(0, 1))]);
    }

    /// A `MatrixRect` excludes its maximum, so the covering rectangle reaches one past the last
    /// point and contains both corners, but nothing beyond them.
    #[test]
    fn a_covering_rect_contains_its_corners_and_nothing_beyond() {
        let rect = matrix_rect_covering([MatrixPoint::new(2, 1), MatrixPoint::new(0, 3)]);

        assert_eq!(
            rect,
            MatrixRect::new(MatrixPoint::new(0, 1), MatrixPoint::new(3, 4))
        );
        assert!(rect.contains(MatrixPoint::new(0, 1)));
        assert!(rect.contains(MatrixPoint::new(2, 3)));
        assert!(!rect.contains(MatrixPoint::new(3, 3)));
        assert!(!rect.contains(MatrixPoint::new(2, 4)));
    }
}
