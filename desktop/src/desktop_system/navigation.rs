use anyhow::Result;
use log::error;

use massive_geometry::{PixelCamera, Size, SizePx};
use massive_scene::prelude::*;

use super::change::{Changes, DesktopChange, set_focus};
use super::fullscreen::fullscreen_scale;
use super::topology::DesktopTopology;
use super::{DesktopSystem, DesktopTarget, Direction, KeyboardFocusReason, LauncherMap};
use crate::projects::{
    FullScreenMode, LaunchProfileId, LauncherMode, MatrixPlacement, ProjectId, RuntimeConfiguration,
};

mod matrix_navigation;
mod zoom_navigation;

use matrix_navigation::MatrixNavigation;

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
            &self.aggregates.configuration,
            &self.navigation_control,
            focused,
            direction,
        ) {
            // Architecture: Totally confusing that set_focus may also change the navigation affinity.
            let mut changes =
                set_focus(Some(plan.candidate.clone()), KeyboardFocusReason::Navigate);
            changes <<= DesktopChange::CommitNavigationAffinity(plan.column_affinity);
            return Ok(changes);
        }

        Ok(Changes::Empty)
    }

    pub(super) fn launcher_removal_focus(
        &self,
        launcher: LaunchProfileId,
        focused: &DesktopTarget,
    ) -> DesktopTarget {
        let matrix_navigation =
            MatrixNavigation::new(&self.aggregates.hierarchy, &self.aggregates.configuration);
        let replacement = [Direction::Right, Direction::Down]
            .into_iter()
            .find_map(|direction| {
                matrix_navigation.navigate_from_launcher(launcher, direction, None)
            })
            .unwrap_or_else(|| {
                DesktopTarget::ProjectMatrix(
                    self.aggregates.hierarchy.project_of_launcher(launcher),
                )
            });

        self.restore_launcher_removal_focus_depth(replacement, focused)
    }

    // Robustness: This parallels `resolve_navigation_focus_target`: both turn a launcher into a
    // concrete focus target. Keep their instance/view selection policies aligned; they may need
    // to be combined once directional navigation also preserves the original focus depth.
    fn restore_launcher_removal_focus_depth(
        &self,
        replacement: DesktopTarget,
        focused: &DesktopTarget,
    ) -> DesktopTarget {
        let DesktopTarget::Launcher(replacement_launcher) = replacement else {
            return replacement;
        };

        let (DesktopTarget::Instance(_) | DesktopTarget::View(_)) = focused else {
            return DesktopTarget::Launcher(replacement_launcher);
        };

        let Some(instance) = self
            .aggregates
            .launchers
            .get(&replacement_launcher)
            .and_then(|launcher| launcher.focus_anchor_instance)
            .filter(|instance| {
                self.aggregates
                    .hierarchy
                    .parent(&DesktopTarget::Instance(*instance))
                    == Some(&DesktopTarget::Launcher(replacement_launcher))
            })
        else {
            return DesktopTarget::Launcher(replacement_launcher);
        };

        let instance = DesktopTarget::Instance(instance);
        match focused {
            DesktopTarget::Instance(_) => instance,
            DesktopTarget::View(_) => self
                .aggregates
                .hierarchy
                .resolve_neighbor_focus_target(&instance),
            // The `let ... else` above limits this branch to instance or view focus.
            _ => unreachable!(),
        }
    }

    /// The focus replacement when a nested `project` leaves its parent's matrix: the
    /// next sibling, else the previous, else the parent project. Only nested
    /// projects are cleared, so a parent always exists.
    pub(super) fn project_removal_focus(&self, project: ProjectId) -> DesktopTarget {
        let parent = self
            .aggregates
            .hierarchy
            .parent_project_of(project)
            .expect("a removed project is nested in its parent's matrix");

        let siblings: Vec<DesktopTarget> = self
            .aggregates
            .configuration
            .slots_ordered(parent)
            .map(|(_, content)| content.target())
            .collect();
        let project_target = DesktopTarget::Project(project);
        let project_index = siblings
            .iter()
            .position(|target| target == &project_target)
            .expect("the removed project is assigned to a sibling slot");
        siblings
            .get(project_index + 1)
            .or_else(|| {
                project_index
                    .checked_sub(1)
                    .and_then(|index| siblings.get(index))
            })
            .cloned()
            .unwrap_or(DesktopTarget::Project(parent))
    }

    pub(super) fn camera_for_target(&self, focus: &DesktopTarget) -> Option<PixelCamera> {
        match focus {
            DesktopTarget::Desktop => {
                // The desktop node has no presenter to frame; its child (the root
                // project) carries the camera.
                self.camera_for_target(self.aggregates.hierarchy.parent(focus)?)
            }
            DesktopTarget::Project(_)
            | DesktopTarget::ProjectHeader(_)
            | DesktopTarget::ProjectMatrix(_)
            | DesktopTarget::Launcher(_) => {
                let transform = self.placement(focus).transform;
                Some(Self::camera_from_placement(transform))
            }
            DesktopTarget::Instance(instance_id) => {
                let transform = self
                    .placement(&DesktopTarget::Instance(*instance_id))
                    .transform;
                // Full Screen Mode (ADR 0014): the view presents window-resolution
                // content scaled by the fullscreen factor into its panel, so the
                // camera dollies in by that factor — the content maps 1:1 onto the
                // screen, the pixel-aligned fullscreen camera. The panel camera
                // would render the content letterboxed at the panel scale.
                //
                // Only while the instance HAS its primary view: the instance
                // target brieflly exists view-less between StartInstance and the
                // view's first submission, and framing that empty panel at the
                // fullscreen distance shows 1/0.75-scaled content for one
                // commit — the "grow, then settle back" bounce on Cmd+T.
                if self.aggregates.instance_full_screen_mode(*instance_id)
                    == FullScreenMode::FullScreen
                    && self.aggregates.view_of_instance(*instance_id).is_some()
                {
                    let content_scale =
                        fullscreen_scale(self.default_panel_size, self.window_state.inner_size);
                    Some(Self::camera_from_placement(transform).with_distance(
                        PixelCamera::pixel_perfect_distance(PixelCamera::DEFAULT_FOVY)
                            * transform.scale
                            * content_scale,
                    ))
                } else {
                    Some(Self::camera_from_placement(transform))
                }
            }
            DesktopTarget::View(_) => {
                self.camera_for_target(self.aggregates.hierarchy.parent(focus)?)
            }
        }
    }

    /// Build a camera that looks at the placement's full transform (translate + rotate),
    /// at the pixel-perfect distance.
    pub(super) fn camera_from_placement(transform: Transform) -> PixelCamera {
        let look_at = Transform::new(transform.translate, transform.rotate, 1.0);
        look_at.to_camera().with_distance(
            PixelCamera::pixel_perfect_distance(PixelCamera::DEFAULT_FOVY) * transform.scale,
        )
    }

    /// The letterboxing camera distance that fits `size` within the window.
    pub(super) fn fit_letterbox_distance(size: Size, window_size: SizePx) -> f64 {
        let (surface_width, surface_height) = window_size.into();
        let scale_x = surface_width as f64 / size.width;
        let scale_y = surface_height as f64 / size.height;
        let fit_scale = scale_x.min(scale_y).max(f64::MIN_POSITIVE);
        PixelCamera::pixel_perfect_distance(PixelCamera::DEFAULT_FOVY) / fit_scale
    }
}

/// Plans a navigation step without mutating navigation state.
///
/// Resolves the candidate target and the column affinity the step would commit.
/// Call `apply_navigation_plan` to commit the affinity once the move is taken.
fn plan_navigation_candidate(
    hierarchy: &DesktopTopology,
    launchers: &LauncherMap,
    configuration: &RuntimeConfiguration,
    navigation_control: &NavigationControl,
    from: &DesktopTarget,
    direction: Direction,
) -> Option<NavigationPlan> {
    let origin = resolve_navigation_origin(hierarchy, from)?;
    let origin_placement = navigation_origin_placement(configuration, origin);
    let column_affinity = navigation_control.plan_column_affinity(direction, origin_placement);
    let matrix_navigation = MatrixNavigation::new(hierarchy, configuration);
    let target = navigate_from_origin(
        matrix_navigation,
        configuration,
        origin,
        direction,
        column_affinity,
    )?;
    let candidate =
        resolve_navigation_focus_target(hierarchy, launchers, configuration, target, direction);
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
            let launcher = hierarchy.launcher_of_instance(*instance_id);
            let index = hierarchy
                .launcher_instances(launcher)
                .position(|instance| instance == *instance_id)?;
            Some(NavigationOrigin::Child { launcher, index })
        }
        DesktopTarget::View(_) => {
            let instance = hierarchy.instance_of_target(from)?;
            resolve_navigation_origin(hierarchy, &DesktopTarget::Instance(instance))
        }
        _ => None,
    }
}

fn navigation_origin_placement(
    configuration: &RuntimeConfiguration,
    origin: NavigationOrigin,
) -> Option<MatrixPlacement> {
    match origin {
        NavigationOrigin::Launcher(launcher_id)
        | NavigationOrigin::Child {
            launcher: launcher_id,
            ..
        } => configuration.placement_of(launcher_id),
    }
}

fn navigate_from_origin(
    matrix_navigation: MatrixNavigation<'_>,
    configuration: &RuntimeConfiguration,
    origin: NavigationOrigin,
    direction: Direction,
    preferred_column: Option<u32>,
) -> Option<DesktopTarget> {
    match origin {
        NavigationOrigin::Launcher(launcher) => {
            matrix_navigation.navigate_from_launcher(launcher, direction, preferred_column)
        }
        NavigationOrigin::Child { launcher, index } => matrix_navigation.navigate_from_child(
            configuration,
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
// Robustness: This parallels `restore_launcher_removal_focus_depth`, which also resolves a
// launcher to an instance or view. They may need to be combined when directional navigation and
// launcher removal use the same focus-depth policy.
fn resolve_navigation_focus_target(
    topology: &DesktopTopology,
    launchers: &LauncherMap,
    configuration: &RuntimeConfiguration,
    target: DesktopTarget,
    direction: Direction,
) -> DesktopTarget {
    let target = match target {
        DesktopTarget::Launcher(launcher_id) => {
            concrete_navigation_target(topology, launchers, configuration, launcher_id, direction)
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
    configuration: &RuntimeConfiguration,
    launcher_id: LaunchProfileId,
    direction: Direction,
) -> DesktopTarget {
    // The mode is a configuration question; the anchor is presentation state and stays
    // on the presenter. A launcher absent from either falls back to itself.
    let Some(launcher) = launchers.get(&launcher_id) else {
        return DesktopTarget::Launcher(launcher_id);
    };
    let mode = configuration
        .launcher(launcher_id)
        .map_or(LauncherMode::default(), |launcher| launcher.mode);
    let focus_anchor_instance = launcher.focus_anchor_instance;

    let instances: Vec<_> = topology.launcher_instances(launcher_id).collect();
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
    fn focused_scaled_content_keeps_its_local_pixel_scale() {
        let scale = 0.25;
        let camera = DesktopSystem::camera_from_placement(Transform::from_scale(scale));

        assert_eq!(
            camera.distance,
            PixelCamera::pixel_perfect_distance(PixelCamera::DEFAULT_FOVY) * scale
        );
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
