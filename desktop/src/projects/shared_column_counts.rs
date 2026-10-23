//! How many columns each project's matrix lays out, so that sibling projects line up on the same
//! columns (ADR 0035). The counts follow from the desktop configuration alone. They are kept current
//! as the configuration changes, so that layout and planning read them in constant time and only
//! the projects whose count changed have to be measured again.

use std::cmp::max;
use std::collections::HashMap;

use super::{MatrixPoint, MatrixRect, Project, ProjectId, RuntimeConfiguration};

/// Every project's shared column count, derived from the desktop configuration.
///
/// It is kept next to the configuration and not inside it, so the configuration stays plain data. A
/// configuration change names the projects whose slots it edited. The desktop system refreshes each
/// of them in an effect phase before layout, when the configuration and the topology of the whole
/// transaction are final, so a refresh reads the final tree instead of keeping a copy of it.
#[derive(Debug, PartialEq)]
pub struct SharedColumnCounts {
    /// Each project's own column count: the columns up to its highest assigned one.
    own: HashMap<ProjectId, u32>,
    /// The number of columns each project's matrix lays out, its shared column count.
    shared: HashMap<ProjectId, u32>,
}

impl SharedColumnCounts {
    /// Resolves every count from scratch, linear in the number of slots. The desktop system starts
    /// with this, and the tests compare the refreshed counts with it.
    pub fn new(configuration: &RuntimeConfiguration) -> Self {
        let mut counts = Self {
            own: HashMap::new(),
            shared: HashMap::new(),
        };
        let mut pending = vec![ProjectId::ROOT];
        while let Some(project) = pending.pop() {
            let record = &configuration[project];
            counts.own.insert(project, own_column_count(record));
            pending.extend(record.nested_projects());
        }
        let root_own = counts.own[&ProjectId::ROOT];
        counts.shared.insert(ProjectId::ROOT, root_own);
        let projects: Vec<ProjectId> = counts.own.keys().copied().collect();
        for project in projects {
            counts.share_among_children(project, configuration, &mut Vec::new());
        }
        counts
    }

    /// The number of columns `project`'s matrix lays out: the largest own column count among the
    /// projects in its parent's matrix. The root has no siblings, so it lays out its own columns.
    pub fn shared_column_count(&self, project: ProjectId) -> u32 {
        *self
            .shared
            .get(&project)
            .expect("a counted project is in the configuration")
    }

    /// The rectangle of `project`'s matrix that a selection extends in and a selected block moves
    /// in: as wide as its shared column count and as tall as its own rows (ADR 0035).
    pub fn shared_matrix_bounds(&self, project: &Project) -> MatrixRect {
        let own = project.matrix_bounds();
        MatrixRect::new(
            own.min,
            MatrixPoint::new(self.shared_column_count(project.id), own.max.y),
        )
    }

    /// Brings the counts up to date after configuration changes edited `project`'s slots, and
    /// returns the projects whose shared column count changed, so that their matrices are measured
    /// again. `parent` is the project whose matrix hosts `project`, `None` for the root.
    ///
    /// The project's own column count is shared among its siblings, so their counts are
    /// recomputed. The project's slots may have gained or lost projects, so its children's counts
    /// are recomputed too. Both read the final configuration, so the refreshes of one transaction
    /// can run in any order. A project that changed back to its former count within those
    /// refreshes is still returned, which costs one measure that changes nothing.
    pub fn refresh(
        &mut self,
        project: ProjectId,
        parent: Option<ProjectId>,
        configuration: &RuntimeConfiguration,
    ) -> Vec<ProjectId> {
        let mut changed = Vec::new();
        let own = own_column_count(&configuration[project]);
        self.own.insert(project, own);
        match parent {
            Some(parent) => self.share_among_children(parent, configuration, &mut changed),
            None => self.set_shared(project, own, &mut changed),
        }
        self.share_among_children(project, configuration, &mut changed);
        changed
    }

    /// Drops the counts of a project that left the configuration.
    pub fn forget(&mut self, project: ProjectId) {
        self.own.remove(&project);
        self.shared.remove(&project);
    }

    /// Gives every child project of `parent` the largest own column count among them.
    fn share_among_children(
        &mut self,
        parent: ProjectId,
        configuration: &RuntimeConfiguration,
        changed: &mut Vec<ProjectId>,
    ) {
        let children = configuration[parent].nested_projects();
        let largest = children.iter().fold(0, |largest, &child| {
            max(largest, self.own_column_count_of(child, configuration))
        });
        for child in children {
            self.set_shared(child, largest, changed);
        }
    }

    /// The own column count of `project`. A project that a change created in a slot is not
    /// refreshed itself, because only the project hosting the slot is touched, so it is counted
    /// when its siblings first need it.
    fn own_column_count_of(
        &mut self,
        project: ProjectId,
        configuration: &RuntimeConfiguration,
    ) -> u32 {
        *self
            .own
            .entry(project)
            .or_insert_with(|| own_column_count(&configuration[project]))
    }

    fn set_shared(&mut self, project: ProjectId, count: u32, changed: &mut Vec<ProjectId>) {
        if self.shared.insert(project, count) != Some(count) {
            changed.push(project);
        }
    }
}

/// The columns `project` holds itself: up to its highest assigned column. An empty project holds
/// one column, because it measures one slot extent, like an empty launcher (ADR 0023).
fn own_column_count(project: &Project) -> u32 {
    project
        .slot_contents()
        .map(|(position, _)| position.column + 1)
        .max()
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use massive_applications::Direction;

    use super::*;
    use crate::desktop_system::change::ConfigurationChange;
    use crate::projects::{
        FullScreenMode, LaunchProfile, LaunchProfileId, LauncherMode, MatrixPoint, MatrixRect,
        Params, ROOT_PROJECT_NAME, SlotAssignment, SlotContent, SlotPosition, matrix_bounds,
        move_landings,
    };

    /// The counts are state derived from the configuration, so they must never drift from it.
    /// Random transactions of configuration changes of every kind refresh the projects the changes
    /// touched and forget the projects they removed, in a random order, as the desktop system's
    /// refresh effects do. After each transaction the counts equal counts resolved from scratch, and every
    /// project whose shared column count changed was reported, because only reported matrices are
    /// measured again.
    #[test]
    fn refreshed_counts_match_counts_resolved_from_scratch() {
        for seed in 1..=4 {
            let mut random = Random(seed);
            let mut root = Project::new(ROOT_PROJECT_NAME.into(), Vec::new());
            root.id = ProjectId::ROOT;
            let mut configuration = RuntimeConfiguration::new(vec![root], None);
            let mut counts = SharedColumnCounts::new(&configuration);

            for _ in 0..500 {
                let before = SharedColumnCounts::new(&configuration);
                let mut touched = Vec::new();
                for _ in 0..=random.below(3) {
                    for change in random_changes(&mut random, &configuration) {
                        touched.extend(change.touched_projects());
                        touched.extend(apply(change, &mut configuration));
                    }
                }
                for index in (1..touched.len()).rev() {
                    touched.swap(index, random.below(index + 1));
                }
                let mut seen = HashSet::new();
                touched.retain(|&project| seen.insert(project));

                let mut reported = HashSet::new();
                for project in touched {
                    if configuration.project(project).is_none() {
                        counts.forget(project);
                        continue;
                    }
                    let parent = parent_of(project, &configuration);
                    reported.extend(counts.refresh(project, parent, &configuration));
                }

                let resolved = SharedColumnCounts::new(&configuration);
                assert_eq!(counts, resolved);
                for (project, count) in &resolved.shared {
                    if before.shared.get(project) != Some(count) {
                        assert!(reported.contains(project), "{project:?} changed unreported");
                    }
                }
            }
        }
    }

    /// One random change, or for an ungrouping the moves that clear its landing slots first, as
    /// the planner orders them.
    fn random_changes(
        random: &mut Random,
        configuration: &RuntimeConfiguration,
    ) -> Vec<ConfigurationChange> {
        let projects = projects_of(configuration);
        let parent = projects[random.below(projects.len())];
        let assigned: Vec<(SlotPosition, SlotContent)> =
            configuration.slots_ordered(parent).collect();
        let free = random_free_slot(random, configuration, parent);
        let assigned_slot = |random: &mut Random| {
            (!assigned.is_empty()).then(|| assigned[random.below(assigned.len())])
        };

        match random.below(8) {
            0 | 1 => free
                .map(|slot| ConfigurationChange::AssignSlot {
                    parent: Some(parent),
                    slot,
                    assignment: SlotAssignment::Launcher {
                        id: LaunchProfileId::new(),
                        profile: LaunchProfile {
                            name: "launcher".into(),
                            mode: LauncherMode::Visor,
                            params: Params::new(),
                            full_screen_mode: FullScreenMode::Regular,
                        },
                    },
                })
                .into_iter()
                .collect(),
            2 => free
                .map(|slot| ConfigurationChange::AssignSlot {
                    parent: Some(parent),
                    slot,
                    assignment: SlotAssignment::Project {
                        id: ProjectId::new(),
                        name: "project".into(),
                    },
                })
                .into_iter()
                .collect(),
            3 => assigned_slot(random)
                .map(|(slot, _)| ConfigurationChange::ClearSlot { parent, slot })
                .into_iter()
                .collect(),
            4 => {
                let dest = projects[random.below(projects.len())];
                let source = assigned_slot(random);
                let dest_slot = random_free_slot(random, configuration, dest);
                match (source, dest_slot) {
                    (Some((source, _)), Some(dest_slot))
                        if configuration.can_move_slot((parent, source), (dest, dest_slot)) =>
                    {
                        vec![ConfigurationChange::MoveSlot {
                            source: (parent, source),
                            dest: (dest, dest_slot),
                        }]
                    }
                    _ => Vec::new(),
                }
            }
            5 => match (assigned_slot(random), assigned_slot(random)) {
                (Some((a, _)), Some((b, _))) if a != b => {
                    vec![ConfigurationChange::SwapSlots { parent, a, b }]
                }
                _ => Vec::new(),
            },
            // A single grouped slot keeps the new project in that slot, so no slot is in its way.
            6 => assigned_slot(random)
                .map(|(slot, _)| ConfigurationChange::Group {
                    parent,
                    slots: vec![slot],
                    project: ProjectId::new(),
                    name: "group".into(),
                })
                .into_iter()
                .collect(),
            _ => match assigned_slot(random) {
                Some((slot, SlotContent::Project(project))) => {
                    let landings: Vec<SlotPosition> = configuration
                        .ungroup_landings(project, slot)
                        .into_iter()
                        .map(|(_, landing)| landing)
                        .collect();
                    let mut changes: Vec<ConfigurationChange> = configuration
                        .ungroup_pushes(parent, slot, &landings)
                        .into_iter()
                        .map(|(source, dest)| ConfigurationChange::MoveSlot {
                            source: (parent, source),
                            dest: (parent, dest),
                        })
                        .collect();
                    changes.push(ConfigurationChange::Ungroup { parent, slot });
                    changes
                }
                Some((slot, SlotContent::Launcher(_))) => {
                    let block = MatrixRect::new(
                        MatrixPoint::new(slot.column, slot.row),
                        MatrixPoint::new(slot.column + 1, slot.row + 1),
                    );
                    let direction = [
                        Direction::Left,
                        Direction::Right,
                        Direction::Up,
                        Direction::Down,
                    ][random.below(4)];
                    let slots = assigned.iter().map(|(slot, _)| *slot);
                    move_landings(block, direction, slots.clone(), matrix_bounds(slots))
                        .filter(|landings| !landings.is_empty())
                        .map(|landings| ConfigurationChange::RelocateSlots { parent, landings })
                        .into_iter()
                        .collect()
                }
                None => Vec::new(),
            },
        }
    }

    /// Applies `change` to the configuration the way the desktop system does, and returns the
    /// projects it removed.
    fn apply(
        change: ConfigurationChange,
        configuration: &mut RuntimeConfiguration,
    ) -> Vec<ProjectId> {
        match change {
            ConfigurationChange::AssignSlot {
                parent,
                slot,
                assignment,
            } => {
                configuration.assign_slot(parent, slot, assignment);
            }
            ConfigurationChange::ClearSlot { parent, slot } => {
                return configuration
                    .clear_slot(parent, slot)
                    .into_iter()
                    .filter_map(|content| match content {
                        SlotContent::Project(project) => Some(project),
                        SlotContent::Launcher(_) => None,
                    })
                    .collect();
            }
            ConfigurationChange::MoveSlot { source, dest } => {
                assert!(configuration.move_slot(source, dest));
            }
            ConfigurationChange::Group {
                parent,
                slots,
                project,
                name,
            } => configuration.group_slots(parent, &slots, project, &name),
            ConfigurationChange::Ungroup { parent, slot } => {
                return vec![configuration.ungroup_slot(parent, slot)];
            }
            ConfigurationChange::SwapSlots { parent, a, b } => {
                configuration.swap_slots(parent, a, b);
            }
            ConfigurationChange::RelocateSlots { parent, landings } => {
                configuration.relocate_slots(parent, &landings);
            }
            ConfigurationChange::SetStartupPath(path) => configuration.set_startup_path(path),
        }
        Vec::new()
    }

    /// The project whose matrix hosts `project`, as the desktop system takes it from the topology.
    fn parent_of(project: ProjectId, configuration: &RuntimeConfiguration) -> Option<ProjectId> {
        projects_of(configuration)
            .into_iter()
            .find(|&parent| configuration[parent].nested_projects().contains(&project))
    }

    fn projects_of(configuration: &RuntimeConfiguration) -> Vec<ProjectId> {
        let mut projects = vec![ProjectId::ROOT];
        let mut index = 0;
        while index < projects.len() {
            projects.extend(configuration[projects[index]].nested_projects());
            index += 1;
        }
        projects
    }

    /// A random unassigned slot of a small matrix, so that slots often collide and columns often
    /// empty out.
    fn random_free_slot(
        random: &mut Random,
        configuration: &RuntimeConfiguration,
        project: ProjectId,
    ) -> Option<SlotPosition> {
        let slot = SlotPosition {
            column: random.below(5) as u32,
            row: random.below(3) as u32,
        };
        configuration
            .content_at(project, slot)
            .is_none()
            .then_some(slot)
    }

    /// A xorshift generator, so that a failing sequence of changes repeats with its seed.
    struct Random(u64);

    impl Random {
        fn below(&mut self, bound: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % bound as u64) as usize
        }
    }
}
