use anyhow::Result;

use log::error;
use massive_geometry::{PixelCamera, Rect, RectPx};
use massive_scene::{ToCamera, Transform};

use super::{DesktopSystem, DesktopTarget, KeyboardFocusReason};
use crate::desktop_system::LauncherMap;
use crate::desktop_system::change::{Changes, DesktopChange, set_focus};
use crate::desktop_system::topology::DesktopTopology;
use crate::projects::{LaunchProfileId, LauncherMode, MatrixPlacement, ProjectId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HorizontalDirection {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerticalDirection {
    Up,
    Down,
}

impl Direction {
    fn horizontal(self) -> Option<HorizontalDirection> {
        match self {
            Direction::Left => Some(HorizontalDirection::Left),
            Direction::Right => Some(HorizontalDirection::Right),
            Direction::Up | Direction::Down => None,
        }
    }

    fn vertical(self) -> Option<VerticalDirection> {
        match self {
            Direction::Up => Some(VerticalDirection::Up),
            Direction::Down => Some(VerticalDirection::Down),
            Direction::Left | Direction::Right => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NavigationControl {
    column_affinity: Option<u32>,
}

impl NavigationControl {
    /// Computes the column affinity a navigation step would produce, without mutating.
    ///
    /// Horizontal moves clear the affinity; the first vertical move latches the origin
    /// column; subsequent vertical moves keep the latched column. The returned value is
    /// also the preferred column to feed into matrix navigation.
    fn plan_column_affinity(
        &self,
        direction: Direction,
        origin: Option<MatrixPlacement>,
    ) -> Option<u32> {
        if direction.horizontal().is_some() {
            return None;
        }

        if direction.vertical().is_some() && self.column_affinity.is_none() {
            return origin.map(|origin| origin.column);
        }

        self.column_affinity
    }

    pub fn commit_column_affinity(&mut self, column_affinity: Option<u32>) {
        self.column_affinity = column_affinity;
    }
}

#[derive(Debug, Clone)]
pub struct NavigationPlan {
    candidate: DesktopTarget,
    column_affinity: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
struct MatrixEntry<K> {
    key: K,
    placement: MatrixPlacement,
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
    /// Plans a navigation command into changes without mutating state.
    ///
    /// Resolves the navigation candidate (and the column affinity the move would commit) from the
    /// current focus and user state, then emits `SetFocus`, `SetNavigationAffinity`, and — when in
    /// overview — `SetUserState` for the resulting overview target. The actual focus change,
    /// affinity commit, and user-state update happen when those changes are applied.
    pub(super) fn plan_navigate(&self, direction: Direction) -> Result<Changes> {
        // If nothing is focused (i.e. the whole window does not have the focused), we probably
        // don't want to do anything and this is perhaps even an error.
        let Some(focused) = self.event_router.focused() else {
            error!("Navigation request without active focus");
            return Ok(Changes::Empty);
        };

        if let Some(plan) = plan_navigation_candidate(
            &self.aggregates.hierarchy,
            &self.aggregates.launchers,
            &self.navigation_control,
            focused,
            direction,
        ) {
            let mut changes =
                set_focus(Some(plan.candidate.clone()), KeyboardFocusReason::Navigate);
            changes += DesktopChange::SetNavigationAffinity(plan.column_affinity);
            return Ok(changes);
        }

        Ok(Changes::Empty)
    }

    pub(super) fn camera_for_target(&self, focus: &DesktopTarget) -> Option<PixelCamera> {
        match focus {
            DesktopTarget::Desktop => {
                let placement = self.placement(&DesktopTarget::Desktop);
                let rect: RectPx = placement.rect.into();
                let rect: Rect = rect.into();
                let size = rect.size();
                // The Desktop is the layout root — its transform is T::default() (IDENTITY),
                // not center-based. Compute the center from the rectangle.
                let center = rect.center();
                let center: Transform = (center.x, center.y, 0.0).into();
                Some(center.to_camera().with_size(size))
            }
            DesktopTarget::Project(_)
            | DesktopTarget::ProjectHeader(_)
            | DesktopTarget::ProjectMatrix(_)
            | DesktopTarget::Launcher(_) => {
                let transform = self.placement(focus).transform;
                let camera_transform: Transform = transform.translate.into();
                Some(camera_transform.to_camera())
            }
            DesktopTarget::Instance(instance_id) => {
                let transform = self
                    .placement(&DesktopTarget::Instance(*instance_id))
                    .transform;
                let transform: Transform = transform.translate.into();
                Some(transform.to_camera())
            }
            DesktopTarget::View(_) => {
                self.camera_for_target(self.aggregates.hierarchy.parent(focus)?)
            }
        }
    }
}

/// Plans a navigation step without mutating navigation state.
///
/// Resolves the candidate target and the column affinity the step would commit.
/// Call `apply_navigation_plan` to commit the affinity once the move is taken.
fn plan_navigation_candidate(
    hierarchy: &DesktopTopology,
    launchers: &LauncherMap,
    navigation_control: &NavigationControl,
    from: &DesktopTarget,
    direction: Direction,
) -> Option<NavigationPlan> {
    let origin = resolve_navigation_origin(hierarchy, from)?;
    let origin_placement = navigation_origin_placement(launchers, origin);
    let column_affinity = navigation_control.plan_column_affinity(direction, origin_placement);
    let target = navigate_from_origin(hierarchy, launchers, origin, direction, column_affinity)?;
    let candidate = normalize_navigation_target(hierarchy, launchers, target, direction);
    Some(NavigationPlan {
        candidate,
        column_affinity,
    })
}

fn resolve_navigation_origin(
    hierarchy: &DesktopTopology,
    from: &DesktopTarget,
) -> Option<NavigationOrigin> {
    match from {
        DesktopTarget::Launcher(launcher_id) => Some(NavigationOrigin::Launcher(*launcher_id)),
        DesktopTarget::Instance(instance_id) => {
            let launcher = match hierarchy.parent(&DesktopTarget::Instance(*instance_id))? {
                DesktopTarget::Launcher(launcher_id) => *launcher_id,
                _ => return None,
            };
            let instances = hierarchy.launcher_instances(launcher);
            let index = instances
                .iter()
                .position(|instance| instance == instance_id)?;
            Some(NavigationOrigin::Child { launcher, index })
        }
        DesktopTarget::View(view_id) => {
            let instance = match hierarchy.parent(&DesktopTarget::View(*view_id))? {
                DesktopTarget::Instance(instance_id) => *instance_id,
                _ => return None,
            };
            resolve_navigation_origin(hierarchy, &DesktopTarget::Instance(instance))
        }
        _ => None,
    }
}

fn navigation_origin_placement(
    launchers: &LauncherMap,
    origin: NavigationOrigin,
) -> Option<MatrixPlacement> {
    match origin {
        NavigationOrigin::Launcher(launcher_id)
        | NavigationOrigin::Child {
            launcher: launcher_id,
            ..
        } => launchers
            .get(&launcher_id)
            .map(|launcher| launcher.placement),
    }
}

fn navigate_from_origin(
    hierarchy: &DesktopTopology,
    launchers: &LauncherMap,
    origin: NavigationOrigin,
    direction: Direction,
    preferred_column: Option<u32>,
) -> Option<DesktopTarget> {
    match origin {
        NavigationOrigin::Launcher(launcher) => {
            navigate_from_launcher(hierarchy, launchers, launcher, direction, preferred_column)
        }
        NavigationOrigin::Child { launcher, index } => navigate_from_child(
            hierarchy,
            launchers,
            launcher,
            index,
            direction,
            preferred_column,
        ),
    }
}

fn navigate_from_child(
    hierarchy: &DesktopTopology,
    launchers: &LauncherMap,
    launcher_id: LaunchProfileId,
    index: usize,
    direction: Direction,
    preferred_column: Option<u32>,
) -> Option<DesktopTarget> {
    let _ = launchers.get(&launcher_id)?;
    let instances = hierarchy.launcher_instances(launcher_id);
    if let Some(horizontal) = direction.horizontal() {
        return horizontal_child_neighbor(&instances, index, horizontal)
            .map(DesktopTarget::Instance)
            .or_else(|| {
                navigate_from_launcher(
                    hierarchy,
                    launchers,
                    launcher_id,
                    direction,
                    preferred_column,
                )
            });
    }

    navigate_from_launcher(
        hierarchy,
        launchers,
        launcher_id,
        direction,
        preferred_column,
    )
}

fn horizontal_child_neighbor(
    instances: &[massive_applications::InstanceId],
    index: usize,
    direction: HorizontalDirection,
) -> Option<massive_applications::InstanceId> {
    match direction {
        HorizontalDirection::Left => (index > 0).then(|| instances[index - 1]),
        HorizontalDirection::Right => (index + 1 < instances.len()).then(|| instances[index + 1]),
    }
}

/// Normalizes a raw navigation result into a concrete, focusable target.
///
/// Matrix navigation may return a `Launcher` shell. This step converts launcher
/// targets into concrete child instances when appropriate, then delegates to the
/// hierarchy to resolve the final focus target (for example, a nested view).
fn normalize_navigation_target(
    topology: &DesktopTopology,
    launchers: &LauncherMap,
    target: DesktopTarget,
    direction: Direction,
) -> DesktopTarget {
    let target = match target {
        DesktopTarget::Launcher(launcher_id) => {
            concrete_navigation_target(topology, launchers, launcher_id, direction)
        }
        _ => target,
    };

    topology.resolve_neighbor_focus_target(&target)
}

/// Chooses a concrete focus target for a launcher.
///
/// If the launcher has instances, returns the preferred instance for the current
/// mode and direction (for example, the visor focus anchor when available).
/// Otherwise, it falls back to the launcher itself.
fn concrete_navigation_target(
    topology: &DesktopTopology,
    launchers: &LauncherMap,
    launcher_id: LaunchProfileId,
    direction: Direction,
) -> DesktopTarget {
    let (mode, focus_anchor_instance) = match launchers.get(&launcher_id) {
        Some(launcher) => (launcher.mode(), launcher.focus_anchor_instance),
        None => return DesktopTarget::Launcher(launcher_id),
    };

    let instances = topology.launcher_instances(launcher_id);
    let preferred_index = match (mode, focus_anchor_instance) {
        (LauncherMode::Visor, Some(focused)) => {
            instances.iter().position(|instance| *instance == focused)
        }
        _ => None,
    };

    if let Some(target_index) =
        select_concrete_instance_index(instances.len(), direction, preferred_index)
    {
        DesktopTarget::Instance(instances[target_index])
    } else {
        DesktopTarget::Launcher(launcher_id)
    }
}

fn navigate_from_launcher(
    hierarchy: &DesktopTopology,
    launchers: &LauncherMap,
    launcher_id: LaunchProfileId,
    direction: Direction,
    preferred_column: Option<u32>,
) -> Option<DesktopTarget> {
    let (project_id, origin_placement) =
        launcher_matrix_position(hierarchy, launchers, launcher_id)?;
    let entries = create_project_matrix_entries(hierarchy, launchers, project_id);
    let target = select_matrix_neighbor(&entries, origin_placement, direction, preferred_column)
        .or_else(|| {
            direction.vertical().and_then(|vertical| {
                cross_project_vertical_neighbor(
                    hierarchy,
                    launchers,
                    project_id,
                    preferred_column.unwrap_or(origin_placement.column),
                    vertical,
                )
            })
        })?;
    Some(DesktopTarget::Launcher(target))
}

fn launcher_matrix_position(
    hierarchy: &DesktopTopology,
    launchers: &LauncherMap,
    launcher_id: LaunchProfileId,
) -> Option<(ProjectId, MatrixPlacement)> {
    let project_id = match hierarchy.parent(&DesktopTarget::Launcher(launcher_id))? {
        DesktopTarget::ProjectMatrix(project_id) => *project_id,
        _ => return None,
    };

    let placement = launchers.get(&launcher_id)?.placement;
    Some((project_id, placement))
}

fn create_project_matrix_entries(
    hierarchy: &DesktopTopology,
    launchers: &LauncherMap,
    project_id: ProjectId,
) -> Vec<MatrixEntry<LaunchProfileId>> {
    hierarchy
        .get_nested(&DesktopTarget::ProjectMatrix(project_id))
        .iter()
        .filter_map(|target| {
            let DesktopTarget::Launcher(launcher_id) = target else {
                return None;
            };
            let placement = launchers.get(launcher_id)?.placement;
            Some(MatrixEntry {
                key: *launcher_id,
                placement,
            })
        })
        .collect()
}

fn cross_project_vertical_neighbor(
    hierarchy: &DesktopTopology,
    launchers: &LauncherMap,
    project_id: ProjectId,
    origin_column: u32,
    direction: VerticalDirection,
) -> Option<LaunchProfileId> {
    let project_targets = hierarchy.get_nested(&DesktopTarget::Desktop);
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
        let entries = create_project_matrix_entries(hierarchy, launchers, *candidate_project);
        if let Some(target) =
            select_cross_project_vertical_entry(&entries, origin_column, direction)
        {
            return Some(target);
        }
    }

    None
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

    #[test]
    fn navigation_control_clears_column_affinity_on_horizontal_navigation() {
        let mut control = NavigationControl::default();

        let vertical = control.plan_column_affinity(Direction::Down, Some((3, 0).into()));
        control.commit_column_affinity(vertical);
        let horizontal = control.plan_column_affinity(Direction::Right, Some((3, 1).into()));
        control.commit_column_affinity(horizontal);
        let next_vertical = control.plan_column_affinity(Direction::Up, Some((1, 1).into()));
        control.commit_column_affinity(next_vertical);

        assert_eq!(vertical, Some(3));
        assert_eq!(horizontal, None);
        assert_eq!(next_vertical, Some(1));
    }

    #[test]
    fn navigation_control_reset_all_clears_affinity() {
        let mut control = NavigationControl::default();

        let initial = control.plan_column_affinity(Direction::Down, Some((4, 0).into()));
        control.commit_column_affinity(initial);
        control.commit_column_affinity(None);
        let vertical = control.plan_column_affinity(Direction::Down, Some((2, 1).into()));
        control.commit_column_affinity(vertical);

        assert_eq!(vertical, Some(2));
    }
}
