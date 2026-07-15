use massive_applications::InstanceId;

use super::{Direction, HorizontalDirection, VerticalDirection};
use crate::MatrixPositions;
use crate::desktop_system::topology::DesktopTopology;
use crate::desktop_system::{DesktopTarget, LauncherMap};
use crate::projects::{LaunchProfileId, MatrixPlacement, ProjectId};

#[derive(Debug, Clone, Copy)]
pub(super) struct MatrixNavigation<'a> {
    hierarchy: &'a DesktopTopology,
    positions: &'a MatrixPositions,
}

#[derive(Debug, Clone, Copy)]
struct MatrixEntry<K> {
    key: K,
    placement: MatrixPlacement,
}

impl<'a> MatrixNavigation<'a> {
    pub(super) fn new(hierarchy: &'a DesktopTopology, positions: &'a MatrixPositions) -> Self {
        Self {
            hierarchy,
            positions,
        }
    }

    pub(super) fn navigate_from_launcher(
        self,
        launcher_id: LaunchProfileId,
        direction: Direction,
        preferred_column: Option<u32>,
    ) -> Option<DesktopTarget> {
        let (project_id, origin_placement) = self.launcher_matrix_position(launcher_id)?;
        let entries = self.create_project_matrix_entries(project_id);
        let target =
            select_matrix_neighbor(&entries, origin_placement, direction, preferred_column)
                .or_else(|| {
                    direction.vertical().and_then(|vertical| {
                        self.cross_project_vertical_neighbor(
                            project_id,
                            preferred_column.unwrap_or(origin_placement.column),
                            vertical,
                        )
                    })
                })?;
        Some(DesktopTarget::Launcher(target))
    }

    pub(super) fn navigate_from_child(
        self,
        launchers: &LauncherMap,
        launcher_id: LaunchProfileId,
        index: usize,
        direction: Direction,
        preferred_column: Option<u32>,
    ) -> Option<DesktopTarget> {
        let _ = launchers.get(&launcher_id)?;
        let instances = self.hierarchy.launcher_instances(launcher_id);
        if let Some(horizontal) = direction.horizontal() {
            return horizontal_child_neighbor(&instances, index, horizontal)
                .map(DesktopTarget::Instance)
                .or_else(|| self.navigate_from_launcher(launcher_id, direction, preferred_column));
        }

        self.navigate_from_launcher(launcher_id, direction, preferred_column)
    }

    fn launcher_matrix_position(
        self,
        launcher_id: LaunchProfileId,
    ) -> Option<(ProjectId, MatrixPlacement)> {
        let project_id = match self
            .hierarchy
            .parent(&DesktopTarget::Launcher(launcher_id))?
        {
            DesktopTarget::ProjectMatrix(project_id) => *project_id,
            _ => return None,
        };

        let placement = *self.positions.get(&launcher_id)?;
        Some((project_id, placement))
    }

    fn create_project_matrix_entries(
        self,
        project_id: ProjectId,
    ) -> Vec<MatrixEntry<LaunchProfileId>> {
        self.hierarchy
            .matrix_launchers(project_id)
            .filter_map(|launcher_id| {
                let placement = *self.positions.get(&launcher_id)?;
                Some(MatrixEntry {
                    key: launcher_id,
                    placement,
                })
            })
            .collect()
    }

    fn cross_project_vertical_neighbor(
        self,
        project_id: ProjectId,
        origin_column: u32,
        direction: VerticalDirection,
    ) -> Option<LaunchProfileId> {
        let project_targets = self.hierarchy.get_nested(&DesktopTarget::Desktop);
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
            VerticalDirection::Up => Box::new(project_ids[..project_index].iter().rev()),
            VerticalDirection::Down => Box::new(project_ids[project_index + 1..].iter()),
        };

        for candidate_project in candidate_projects {
            let entries = self.create_project_matrix_entries(*candidate_project);
            if let Some(target) =
                select_cross_project_vertical_entry(&entries, origin_column, direction)
            {
                return Some(target);
            }
        }

        None
    }
}

fn horizontal_child_neighbor(
    instances: &[InstanceId],
    index: usize,
    direction: HorizontalDirection,
) -> Option<InstanceId> {
    match direction {
        HorizontalDirection::Left => (index > 0).then(|| instances[index - 1]),
        HorizontalDirection::Right => (index + 1 < instances.len()).then(|| instances[index + 1]),
    }
}

fn select_matrix_neighbor<K: Copy>(
    entries: &[MatrixEntry<K>],
    origin: MatrixPlacement,
    direction: Direction,
    preferred_column: Option<u32>,
) -> Option<K> {
    if let Some(horizontal) = direction.horizontal() {
        return select_row_neighbor(entries, origin, horizontal);
    }

    if let Some(vertical) = direction.vertical() {
        return select_column_neighbor(
            entries,
            origin.row,
            preferred_column.unwrap_or(origin.column),
            vertical,
        );
    }

    None
}

fn select_row_neighbor<K: Copy>(
    entries: &[MatrixEntry<K>],
    origin: MatrixPlacement,
    direction: HorizontalDirection,
) -> Option<K> {
    match direction {
        HorizontalDirection::Left => entries
            .iter()
            .filter(|entry| {
                entry.placement.row == origin.row && entry.placement.column < origin.column
            })
            .max_by_key(|entry| entry.placement.column)
            .map(|entry| entry.key),
        HorizontalDirection::Right => entries
            .iter()
            .filter(|entry| {
                entry.placement.row == origin.row && entry.placement.column > origin.column
            })
            .min_by_key(|entry| entry.placement.column)
            .map(|entry| entry.key),
    }
}

fn select_column_neighbor<K: Copy>(
    entries: &[MatrixEntry<K>],
    origin_row: u32,
    column: u32,
    direction: VerticalDirection,
) -> Option<K> {
    let target_row = match direction {
        VerticalDirection::Up => entries
            .iter()
            .filter(|entry| entry.placement.row < origin_row)
            .map(|entry| entry.placement.row)
            .max()?,
        VerticalDirection::Down => entries
            .iter()
            .filter(|entry| entry.placement.row > origin_row)
            .map(|entry| entry.placement.row)
            .min()?,
    };

    entries
        .iter()
        .filter(|entry| entry.placement.row == target_row)
        .min_by_key(|entry| {
            let distance = u32::abs_diff(entry.placement.column, column);
            (distance, entry.placement.column)
        })
        .map(|entry| entry.key)
}

fn select_cross_project_vertical_entry<K: Copy>(
    entries: &[MatrixEntry<K>],
    origin_column: u32,
    direction: VerticalDirection,
) -> Option<K> {
    if entries.is_empty() {
        return None;
    }

    if let Some(exact_column) = select_column_boundary(entries, origin_column, direction) {
        return Some(exact_column);
    }

    select_row_boundary_nearest_column(entries, origin_column, direction)
}

fn select_column_boundary<K: Copy>(
    entries: &[MatrixEntry<K>],
    column: u32,
    direction: VerticalDirection,
) -> Option<K> {
    match direction {
        VerticalDirection::Up => entries
            .iter()
            .filter(|entry| entry.placement.column == column)
            .max_by_key(|entry| entry.placement.row)
            .map(|entry| entry.key),
        VerticalDirection::Down => entries
            .iter()
            .filter(|entry| entry.placement.column == column)
            .min_by_key(|entry| entry.placement.row)
            .map(|entry| entry.key),
    }
}

fn select_row_boundary_nearest_column<K: Copy>(
    entries: &[MatrixEntry<K>],
    origin_column: u32,
    direction: VerticalDirection,
) -> Option<K> {
    let boundary_row = match direction {
        VerticalDirection::Up => entries.iter().map(|entry| entry.placement.row).max()?,
        VerticalDirection::Down => entries.iter().map(|entry| entry.placement.row).min()?,
    };

    entries
        .iter()
        .filter(|entry| entry.placement.row == boundary_row)
        .min_by_key(|entry| {
            let distance = u32::abs_diff(entry.placement.column, origin_column);
            (distance, entry.placement.column)
        })
        .map(|entry| entry.key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_horizontal_navigation_skips_empty_cells() {
        let entries = sample_entries();

        let left = select_matrix_neighbor(&entries, (2, 0).into(), Direction::Left, None);
        let right = select_matrix_neighbor(&entries, (0, 0).into(), Direction::Right, None);

        assert_eq!(left, Some(1));
        assert_eq!(right, Some(2));
    }

    #[test]
    fn matrix_vertical_navigation_skips_empty_cells() {
        let entries = sample_entries();

        let down = select_matrix_neighbor(&entries, (0, 0).into(), Direction::Down, None);
        let up = select_matrix_neighbor(&entries, (0, 2).into(), Direction::Up, None);

        assert_eq!(down, Some(3));
        assert_eq!(up, Some(1));
    }

    #[test]
    fn row_neighbor_returns_none_when_no_candidate_exists() {
        let entries = sample_entries();

        let left = select_row_neighbor(&entries, (0, 0).into(), HorizontalDirection::Left);
        let right = select_row_neighbor(&entries, (2, 2).into(), HorizontalDirection::Right);

        assert_eq!(left, None);
        assert_eq!(right, None);
    }

    #[test]
    fn side_row_neighbor_selection_works_for_horizontal_navigation() {
        let entries = vec![
            MatrixEntry {
                key: 1,
                placement: (2, 1).into(),
            },
            MatrixEntry {
                key: 2,
                placement: (4, 1).into(),
            },
        ];

        let side = select_row_neighbor(&entries, (2, 1).into(), HorizontalDirection::Right);

        assert_eq!(side, Some(2));
    }

    #[test]
    fn cross_project_vertical_prefers_same_column_boundary() {
        let entries = vec![
            MatrixEntry {
                key: 10,
                placement: (1, 0).into(),
            },
            MatrixEntry {
                key: 20,
                placement: (3, 2).into(),
            },
            MatrixEntry {
                key: 30,
                placement: (1, 4).into(),
            },
        ];

        let up = select_cross_project_vertical_entry(&entries, 1, VerticalDirection::Up);
        let down = select_cross_project_vertical_entry(&entries, 1, VerticalDirection::Down);

        assert_eq!(up, Some(30));
        assert_eq!(down, Some(10));
    }

    #[test]
    fn cross_project_vertical_falls_back_to_nearest_column_on_boundary_row() {
        let entries = vec![
            MatrixEntry {
                key: 10,
                placement: (0, 1).into(),
            },
            MatrixEntry {
                key: 20,
                placement: (4, 1).into(),
            },
            MatrixEntry {
                key: 30,
                placement: (2, 3).into(),
            },
            MatrixEntry {
                key: 40,
                placement: (6, 3).into(),
            },
        ];

        let down = select_cross_project_vertical_entry(&entries, 3, VerticalDirection::Down);
        let up = select_cross_project_vertical_entry(&entries, 5, VerticalDirection::Up);

        assert_eq!(down, Some(20));
        assert_eq!(up, Some(40));
    }

    #[test]
    fn matrix_vertical_navigation_uses_preferred_column_when_provided() {
        let entries = vec![
            MatrixEntry {
                key: 1,
                placement: (0, 0).into(),
            },
            MatrixEntry {
                key: 2,
                placement: (2, 0).into(),
            },
            MatrixEntry {
                key: 3,
                placement: (0, 2).into(),
            },
            MatrixEntry {
                key: 4,
                placement: (2, 2).into(),
            },
        ];

        let up = select_matrix_neighbor(&entries, (0, 2).into(), Direction::Up, Some(2));

        assert_eq!(up, Some(2));
    }

    #[test]
    fn matrix_vertical_navigation_uses_next_non_empty_row_and_nearest_column() {
        let entries = vec![
            MatrixEntry {
                key: 1,
                placement: (0, 0).into(),
            },
            MatrixEntry {
                key: 2,
                placement: (3, 1).into(),
            },
            MatrixEntry {
                key: 3,
                placement: (1, 2).into(),
            },
        ];

        let down = select_matrix_neighbor(&entries, (0, 0).into(), Direction::Down, None);

        assert_eq!(down, Some(2));
    }

    fn sample_entries() -> Vec<MatrixEntry<usize>> {
        vec![
            MatrixEntry {
                key: 1,
                placement: (0, 0).into(),
            },
            MatrixEntry {
                key: 2,
                placement: (2, 0).into(),
            },
            MatrixEntry {
                key: 3,
                placement: (0, 2).into(),
            },
            MatrixEntry {
                key: 4,
                placement: (2, 2).into(),
            },
            MatrixEntry {
                key: 5,
                placement: (1, 3).into(),
            },
        ]
    }
}
