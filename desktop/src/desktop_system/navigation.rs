use anyhow::Result;

use log::error;
use massive_geometry::{PixelCamera, Rect, RectPx};
use massive_scene::{ToCamera, Transform};

use super::{DesktopSystem, DesktopTarget, KeyboardFocusReason};
use crate::MatrixPositions;
use crate::desktop_system::LauncherMap;
use crate::desktop_system::change::{Changes, DesktopChange, set_focus};
use crate::desktop_system::topology::DesktopTopology;
use crate::projects::{LaunchProfileId, LauncherMode, MatrixPlacement, ProjectId};

mod matrix_navigation;
mod zoom_navigation;

use matrix_navigation::MatrixNavigation;
pub(crate) use zoom_navigation::focus_depth_from_target;

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
        let Some(focused) = self.event_router.keyboard_focus() else {
            error!("Navigation request without active focus");
            return Ok(Changes::Empty);
        };

        if let Some(plan) = plan_navigation_candidate(
            &self.aggregates.hierarchy,
            &self.aggregates.launchers,
            &self.aggregates.matrix_positions,
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

    pub(super) fn launcher_removal_focus(&self, launcher: LaunchProfileId) -> DesktopTarget {
        let matrix_navigation = MatrixNavigation::new(
            &self.aggregates.hierarchy,
            &self.aggregates.matrix_positions,
        );
        [Direction::Right, Direction::Down]
            .into_iter()
            .find_map(|direction| {
                matrix_navigation.navigate_from_launcher(launcher, direction, None)
            })
            .unwrap_or_else(|| {
                DesktopTarget::ProjectMatrix(
                    self.aggregates.hierarchy.project_of_launcher(launcher),
                )
            })
    }

    pub(super) fn project_removal_focus(&self, project: ProjectId) -> DesktopTarget {
        let project_target = DesktopTarget::Project(project);
        let projects = self
            .aggregates
            .hierarchy
            .get_nested(&DesktopTarget::Desktop);
        let project_index = projects
            .iter()
            .position(|target| target == &project_target)
            .expect("Project missing from desktop hierarchy");
        projects
            .get(project_index + 1)
            .cloned()
            .unwrap_or(DesktopTarget::Desktop)
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
    matrix_positions: &MatrixPositions,
    navigation_control: &NavigationControl,
    from: &DesktopTarget,
    direction: Direction,
) -> Option<NavigationPlan> {
    let origin = resolve_navigation_origin(hierarchy, from)?;
    let origin_placement = navigation_origin_placement(matrix_positions, origin);
    let column_affinity = navigation_control.plan_column_affinity(direction, origin_placement);
    let matrix_navigation = MatrixNavigation::new(hierarchy, matrix_positions);
    let target = navigate_from_origin(
        matrix_navigation,
        launchers,
        origin,
        direction,
        column_affinity,
    )?;
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
    matrix_positions: &MatrixPositions,
    origin: NavigationOrigin,
) -> Option<MatrixPlacement> {
    match origin {
        NavigationOrigin::Launcher(launcher_id)
        | NavigationOrigin::Child {
            launcher: launcher_id,
            ..
        } => matrix_positions.get(&launcher_id).copied(),
    }
}

fn navigate_from_origin(
    matrix_navigation: MatrixNavigation<'_>,
    launchers: &LauncherMap,
    origin: NavigationOrigin,
    direction: Direction,
    preferred_column: Option<u32>,
) -> Option<DesktopTarget> {
    match origin {
        NavigationOrigin::Launcher(launcher) => {
            matrix_navigation.navigate_from_launcher(launcher, direction, preferred_column)
        }
        NavigationOrigin::Child { launcher, index } => matrix_navigation.navigate_from_child(
            launchers,
            launcher,
            index,
            direction,
            preferred_column,
        ),
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

#[cfg(test)]
mod tests {
    use super::*;

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
