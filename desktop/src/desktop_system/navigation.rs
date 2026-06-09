use massive_geometry::{PixelCamera, Rect, RectPx};
use massive_scene::{ToCamera, Transform};

use super::{DesktopSystem, DesktopTarget};
use crate::projects::{LaunchProfileId, LauncherMode, ProjectId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy)]
struct MatrixEntry<K> {
    key: K,
    column: u32,
    row: u32,
}

#[derive(Debug, Clone, Copy)]
enum NavigationOrigin {
    Launcher(LaunchProfileId),
    Child {
        launcher: LaunchProfileId,
        index: usize,
    },
}

impl DesktopSystem {
    pub(super) fn camera_for_focus(&self, focus: &DesktopTarget) -> Option<PixelCamera> {
        match focus {
            DesktopTarget::Desktop => {
                let placement = self.placement(&DesktopTarget::Desktop)?;
                let rect: RectPx = placement.rect.into();
                let rect: Rect = rect.into();
                let size = rect.size();
                // The Desktop is the layout root — its transform is T::default() (IDENTITY),
                // not center-based. Compute the center from the rect.
                let center = rect.center();
                let center: Transform = (center.x, center.y, 0.0).into();
                Some(center.to_camera().with_size(size))
            }
            DesktopTarget::Project(_)
            | DesktopTarget::ProjectHeader(_)
            | DesktopTarget::ProjectMatrix(_)
            | DesktopTarget::Launcher(_) => {
                let transform = self.placement(focus)?.transform;
                let camera_transform: Transform = transform.translate.into();
                Some(camera_transform.to_camera())
            }
            DesktopTarget::Instance(instance_id) => {
                let transform = self
                    .placement(&DesktopTarget::Instance(*instance_id))?
                    .transform;
                let transform: Transform = transform.translate.into();
                Some(transform.to_camera())
            }
            DesktopTarget::View(_) => {
                self.camera_for_focus(self.aggregates.hierarchy.parent(focus)?)
            }
        }
    }

    pub(super) fn locate_navigation_candidate(
        &self,
        from: &DesktopTarget,
        direction: Direction,
    ) -> Option<DesktopTarget> {
        let origin = self.resolve_navigation_origin(from)?;
        let target = self.navigate_from_origin(origin, direction)?;
        Some(self.normalize_navigation_target(target, direction))
    }

    fn resolve_navigation_origin(&self, from: &DesktopTarget) -> Option<NavigationOrigin> {
        match from {
            DesktopTarget::Launcher(launcher_id) => Some(NavigationOrigin::Launcher(*launcher_id)),
            DesktopTarget::Instance(instance_id) => {
                let launcher = match self
                    .aggregates
                    .hierarchy
                    .parent(&DesktopTarget::Instance(*instance_id))?
                {
                    DesktopTarget::Launcher(launcher_id) => *launcher_id,
                    _ => return None,
                };
                let instances = self.aggregates.launcher_instance_ids(launcher);
                let index = instances
                    .iter()
                    .position(|instance| instance == instance_id)?;
                Some(NavigationOrigin::Child { launcher, index })
            }
            DesktopTarget::View(view_id) => {
                let instance = match self
                    .aggregates
                    .hierarchy
                    .parent(&DesktopTarget::View(*view_id))?
                {
                    DesktopTarget::Instance(instance_id) => *instance_id,
                    _ => return None,
                };
                self.resolve_navigation_origin(&DesktopTarget::Instance(instance))
            }
            _ => None,
        }
    }

    fn navigate_from_origin(
        &self,
        origin: NavigationOrigin,
        direction: Direction,
    ) -> Option<DesktopTarget> {
        match origin {
            NavigationOrigin::Launcher(launcher) => {
                self.navigate_from_launcher(launcher, None, direction)
            }
            NavigationOrigin::Child { launcher, index } => {
                self.navigate_from_launcher(launcher, Some(index), direction)
            }
        }
    }

    fn navigate_from_launcher(
        &self,
        launcher_id: LaunchProfileId,
        child_index: Option<usize>,
        direction: Direction,
    ) -> Option<DesktopTarget> {
        let _ = self.aggregates.launchers.get(&launcher_id)?;
        let instances = self.aggregates.launcher_instance_ids(launcher_id);

        if let Some(index) = child_index {
            return self.navigate_from_child(launcher_id, &instances, index, direction);
        }

        match direction {
            Direction::Left => {
                if let Some(last) = instances.last() {
                    return Some(DesktopTarget::Instance(*last));
                }
                self.matrix_navigation_from_launcher(launcher_id, direction)
            }
            Direction::Right => {
                if let Some(first) = instances.first() {
                    return Some(DesktopTarget::Instance(*first));
                }
                self.matrix_navigation_from_launcher(launcher_id, direction)
            }
            Direction::Up | Direction::Down => {
                self.matrix_navigation_from_launcher(launcher_id, direction)
            }
        }
    }

    fn navigate_from_child(
        &self,
        launcher_id: LaunchProfileId,
        instances: &[massive_applications::InstanceId],
        index: usize,
        direction: Direction,
    ) -> Option<DesktopTarget> {
        match direction {
            Direction::Left | Direction::Right => self
                .horizontal_child_neighbor(instances, index, direction)
                .map(DesktopTarget::Instance)
                .or_else(|| self.matrix_navigation_from_launcher(launcher_id, direction)),
            Direction::Up | Direction::Down => {
                self.matrix_navigation_from_launcher(launcher_id, direction)
            }
        }
    }

    fn horizontal_child_neighbor(
        &self,
        instances: &[massive_applications::InstanceId],
        index: usize,
        direction: Direction,
    ) -> Option<massive_applications::InstanceId> {
        match direction {
            Direction::Left => (index > 0).then(|| instances[index - 1]),
            Direction::Right => (index + 1 < instances.len()).then(|| instances[index + 1]),
            Direction::Up | Direction::Down => None,
        }
    }

    fn normalize_navigation_target(
        &self,
        target: DesktopTarget,
        direction: Direction,
    ) -> DesktopTarget {
        let target = match target {
            DesktopTarget::Launcher(launcher_id) => {
                self.concrete_navigation_target(launcher_id, direction)
            }
            _ => target,
        };

        self.aggregates
            .hierarchy
            .resolve_neighbor_focus_target(&target)
    }

    fn concrete_navigation_target(
        &self,
        launcher_id: LaunchProfileId,
        direction: Direction,
    ) -> DesktopTarget {
        let (mode, focus_anchor_instance) = match self.aggregates.launchers.get(&launcher_id) {
            Some(launcher) => (launcher.mode(), launcher.focus_anchor_instance()),
            None => return DesktopTarget::Launcher(launcher_id),
        };

        let instances = self.aggregates.launcher_instance_ids(launcher_id);
        let preferred_index = match (mode, focus_anchor_instance) {
            (LauncherMode::Visor, Some(focused)) => {
                instances.iter().position(|instance| *instance == focused)
            }
            _ => None,
        };

        let Some(target_index) =
            select_concrete_instance_index(instances.len(), direction, preferred_index)
        else {
            return DesktopTarget::Launcher(launcher_id);
        };

        DesktopTarget::Instance(instances[target_index])
    }

    fn matrix_navigation_from_launcher(
        &self,
        launcher_id: LaunchProfileId,
        direction: Direction,
    ) -> Option<DesktopTarget> {
        let (project_id, origin_column, origin_row) = self.launcher_matrix_position(launcher_id)?;
        let entries = self.project_matrix_entries(project_id);
        let target = select_matrix_neighbor(&entries, origin_column, origin_row, direction)
            .or_else(|| {
                if matches!(direction, Direction::Up | Direction::Down) {
                    self.cross_project_vertical_neighbor(project_id, origin_column, direction)
                } else {
                    None
                }
            })?;
        Some(DesktopTarget::Launcher(target))
    }

    fn launcher_matrix_position(
        &self,
        launcher_id: LaunchProfileId,
    ) -> Option<(ProjectId, u32, u32)> {
        let project_id = match self
            .aggregates
            .hierarchy
            .parent(&DesktopTarget::Launcher(launcher_id))?
        {
            DesktopTarget::ProjectMatrix(project_id) => *project_id,
            _ => return None,
        };

        let placement = self.aggregates.launchers.get(&launcher_id)?.placement;
        Some((project_id, placement.column, placement.row))
    }

    fn project_matrix_entries(&self, project_id: ProjectId) -> Vec<MatrixEntry<LaunchProfileId>> {
        self.aggregates
            .hierarchy
            .get_nested(&DesktopTarget::ProjectMatrix(project_id))
            .iter()
            .filter_map(|target| {
                let DesktopTarget::Launcher(launcher_id) = target else {
                    return None;
                };
                let placement = self.aggregates.launchers.get(launcher_id)?.placement;
                Some(MatrixEntry {
                    key: *launcher_id,
                    column: placement.column,
                    row: placement.row,
                })
            })
            .collect()
    }

    fn cross_project_vertical_neighbor(
        &self,
        project_id: ProjectId,
        origin_column: u32,
        direction: Direction,
    ) -> Option<LaunchProfileId> {
        let project_targets = self
            .aggregates
            .hierarchy
            .get_nested(&DesktopTarget::Desktop);
        let project_ids: Vec<_> = project_targets
            .iter()
            .filter_map(|target| {
                let DesktopTarget::Project(id) = target else {
                    return None;
                };
                Some(*id)
            })
            .collect();

        let project_index = project_ids.iter().position(|id| *id == project_id)?;

        let candidate_projects: Box<dyn Iterator<Item = &ProjectId>> = match direction {
            Direction::Up => Box::new(project_ids[..project_index].iter().rev()),
            Direction::Down => Box::new(project_ids[project_index + 1..].iter()),
            Direction::Left | Direction::Right => return None,
        };

        for candidate_project in candidate_projects {
            let entries = self.project_matrix_entries(*candidate_project);
            if let Some(target) =
                select_cross_project_vertical_entry(&entries, origin_column, direction)
            {
                return Some(target);
            }
        }

        None
    }
}

fn select_matrix_neighbor<K: Copy>(
    entries: &[MatrixEntry<K>],
    origin_column: u32,
    origin_row: u32,
    direction: Direction,
) -> Option<K> {
    match direction {
        Direction::Left | Direction::Right => {
            select_row_neighbor(entries, origin_row, origin_column, direction)
        }
        Direction::Up | Direction::Down => {
            select_column_neighbor(entries, origin_column, origin_row, direction)
        }
    }
}

fn select_row_neighbor<K: Copy>(
    entries: &[MatrixEntry<K>],
    row: u32,
    origin_column: u32,
    direction: Direction,
) -> Option<K> {
    match direction {
        Direction::Left => entries
            .iter()
            .filter(|entry| entry.row == row && entry.column < origin_column)
            .max_by_key(|entry| entry.column)
            .map(|entry| entry.key),
        Direction::Right => entries
            .iter()
            .filter(|entry| entry.row == row && entry.column > origin_column)
            .min_by_key(|entry| entry.column)
            .map(|entry| entry.key),
        Direction::Up | Direction::Down => None,
    }
}

fn select_concrete_instance_index(
    instance_count: usize,
    direction: Direction,
    preferred_index: Option<usize>,
) -> Option<usize> {
    if instance_count == 0 {
        return None;
    }

    if let Some(preferred_index) = preferred_index
        && preferred_index < instance_count
    {
        return Some(preferred_index);
    }

    match direction {
        Direction::Left => Some(instance_count - 1),
        Direction::Right | Direction::Up | Direction::Down => Some(0),
    }
}

fn select_column_neighbor<K: Copy>(
    entries: &[MatrixEntry<K>],
    column: u32,
    origin_row: u32,
    direction: Direction,
) -> Option<K> {
    match direction {
        Direction::Up => entries
            .iter()
            .filter(|entry| entry.column == column && entry.row < origin_row)
            .max_by_key(|entry| entry.row)
            .map(|entry| entry.key),
        Direction::Down => entries
            .iter()
            .filter(|entry| entry.column == column && entry.row > origin_row)
            .min_by_key(|entry| entry.row)
            .map(|entry| entry.key),
        Direction::Left | Direction::Right => None,
    }
}

fn select_cross_project_vertical_entry<K: Copy>(
    entries: &[MatrixEntry<K>],
    origin_column: u32,
    direction: Direction,
) -> Option<K> {
    if entries.is_empty() {
        return None;
    }

    match direction {
        Direction::Up | Direction::Down => {
            if let Some(exact_column) = select_column_boundary(entries, origin_column, direction) {
                return Some(exact_column);
            }
            select_row_boundary_nearest_column(entries, origin_column, direction)
        }
        Direction::Left | Direction::Right => None,
    }
}

fn select_column_boundary<K: Copy>(
    entries: &[MatrixEntry<K>],
    column: u32,
    direction: Direction,
) -> Option<K> {
    match direction {
        Direction::Up => entries
            .iter()
            .filter(|entry| entry.column == column)
            .max_by_key(|entry| entry.row)
            .map(|entry| entry.key),
        Direction::Down => entries
            .iter()
            .filter(|entry| entry.column == column)
            .min_by_key(|entry| entry.row)
            .map(|entry| entry.key),
        Direction::Left | Direction::Right => None,
    }
}

fn select_row_boundary_nearest_column<K: Copy>(
    entries: &[MatrixEntry<K>],
    origin_column: u32,
    direction: Direction,
) -> Option<K> {
    let boundary_row = match direction {
        Direction::Up => entries.iter().map(|entry| entry.row).max()?,
        Direction::Down => entries.iter().map(|entry| entry.row).min()?,
        Direction::Left | Direction::Right => return None,
    };

    entries
        .iter()
        .filter(|entry| entry.row == boundary_row)
        .min_by_key(|entry| {
            let distance = u32::abs_diff(entry.column, origin_column);
            (distance, entry.column)
        })
        .map(|entry| entry.key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entries() -> Vec<MatrixEntry<usize>> {
        vec![
            MatrixEntry {
                key: 1,
                column: 0,
                row: 0,
            },
            MatrixEntry {
                key: 2,
                column: 2,
                row: 0,
            },
            MatrixEntry {
                key: 3,
                column: 0,
                row: 2,
            },
            MatrixEntry {
                key: 4,
                column: 2,
                row: 2,
            },
            MatrixEntry {
                key: 5,
                column: 1,
                row: 3,
            },
        ]
    }

    #[test]
    fn matrix_horizontal_navigation_skips_empty_cells() {
        let entries = sample_entries();

        let left = select_matrix_neighbor(&entries, 2, 0, Direction::Left);
        let right = select_matrix_neighbor(&entries, 0, 0, Direction::Right);

        assert_eq!(left, Some(1));
        assert_eq!(right, Some(2));
    }

    #[test]
    fn matrix_vertical_navigation_skips_empty_cells() {
        let entries = sample_entries();

        let down = select_matrix_neighbor(&entries, 0, 0, Direction::Down);
        let up = select_matrix_neighbor(&entries, 0, 2, Direction::Up);

        assert_eq!(down, Some(3));
        assert_eq!(up, Some(1));
    }

    #[test]
    fn row_neighbor_returns_none_when_no_candidate_exists() {
        let entries = sample_entries();

        let left = select_row_neighbor(&entries, 0, 0, Direction::Left);
        let right = select_row_neighbor(&entries, 2, 2, Direction::Right);

        assert_eq!(left, None);
        assert_eq!(right, None);
    }

    #[test]
    fn concrete_instance_selection_prefers_directional_edge() {
        assert_eq!(
            select_concrete_instance_index(3, Direction::Left, None),
            Some(2)
        );
        assert_eq!(
            select_concrete_instance_index(3, Direction::Right, None),
            Some(0)
        );
        assert_eq!(
            select_concrete_instance_index(3, Direction::Up, None),
            Some(0)
        );
        assert_eq!(
            select_concrete_instance_index(3, Direction::Down, None),
            Some(0)
        );
    }

    #[test]
    fn concrete_instance_selection_returns_none_for_empty_launcher() {
        assert_eq!(
            select_concrete_instance_index(0, Direction::Left, None),
            None
        );
    }

    #[test]
    fn concrete_instance_selection_prefers_focus_anchor_when_available() {
        assert_eq!(
            select_concrete_instance_index(4, Direction::Left, Some(2)),
            Some(2)
        );
    }

    #[test]
    fn concrete_instance_selection_ignores_invalid_focus_anchor() {
        assert_eq!(
            select_concrete_instance_index(2, Direction::Right, Some(7)),
            Some(0)
        );
    }

    #[test]
    fn side_row_neighbor_selection_works_for_horizontal_navigation() {
        let entries = vec![
            MatrixEntry {
                key: 1,
                column: 2,
                row: 1,
            },
            MatrixEntry {
                key: 2,
                column: 4,
                row: 1,
            },
        ];

        let side = select_row_neighbor(&entries, 1, 2, Direction::Right);

        assert_eq!(side, Some(2));
    }

    #[test]
    fn cross_project_vertical_prefers_same_column_boundary() {
        let entries = vec![
            MatrixEntry {
                key: 10,
                column: 1,
                row: 0,
            },
            MatrixEntry {
                key: 20,
                column: 3,
                row: 2,
            },
            MatrixEntry {
                key: 30,
                column: 1,
                row: 4,
            },
        ];

        let up = select_cross_project_vertical_entry(&entries, 1, Direction::Up);
        let down = select_cross_project_vertical_entry(&entries, 1, Direction::Down);

        assert_eq!(up, Some(30));
        assert_eq!(down, Some(10));
    }

    #[test]
    fn cross_project_vertical_falls_back_to_nearest_column_on_boundary_row() {
        let entries = vec![
            MatrixEntry {
                key: 10,
                column: 0,
                row: 1,
            },
            MatrixEntry {
                key: 20,
                column: 4,
                row: 1,
            },
            MatrixEntry {
                key: 30,
                column: 2,
                row: 3,
            },
            MatrixEntry {
                key: 40,
                column: 6,
                row: 3,
            },
        ];

        let down = select_cross_project_vertical_entry(&entries, 3, Direction::Down);
        let up = select_cross_project_vertical_entry(&entries, 5, Direction::Up);

        assert_eq!(down, Some(20));
        assert_eq!(up, Some(40));
    }
}
