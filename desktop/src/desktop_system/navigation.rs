use anyhow::Result;
use log::error;

use massive_geometry::{PixelCamera, Size, SizePx};
use massive_scene::prelude::*;

use super::change::{Changes, DesktopChange, set_focus};
use super::fullscreen::fullscreen_scale;
use super::topology::DesktopTopology;
use super::{DesktopSystem, DesktopTarget, Direction, KeyboardFocusReason, LauncherMap};
use crate::projects::{
    FullScreenMode, LaunchProfileId, LauncherMode, MatrixPlacement, ProjectId,
    RuntimeConfiguration, SlotContent,
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
    MatrixSlot {
        project: ProjectId,
        placement: MatrixPlacement,
    },
    Child {
        launcher: LaunchProfileId,
        index: usize,
    },
}

impl DesktopSystem {
    /// Plans a navigation command into changes without mutating state.
    ///
    /// Navigation depends only on the keyboard-focused target, never on the zoom level; it
    /// commits focus and column affinity, and the camera frames the new focus at the same zoom
    /// level (ADR 0018).
    pub(super) fn plan_navigate(&self, direction: Direction) -> Result<Changes> {
        // If nothing is focused (i.e. the whole window does not have the focused), we probably
        // don't want to do anything and this is perhaps even an error.
        let Some(focused) = self.event_router.keyboard_focus() else {
            error!("Navigation request without active focus");
            return Ok(Changes::Empty);
        };

        let Some(plan) = plan_navigation_candidate(
            &self.aggregates.hierarchy,
            &self.aggregates.launchers,
            &self.aggregates.configuration,
            &self.navigation_control,
            focused,
            direction,
        ) else {
            return Ok(Changes::Empty);
        };

        // Architecture: Totally confusing that set_focus may also change the navigation affinity.
        let mut changes = set_focus(Some(plan.candidate), KeyboardFocusReason::Navigate);
        changes <<= DesktopChange::CommitNavigationAffinity(plan.column_affinity);
        Ok(changes)
    }

    /// Lands focus on a slot's content without a direction: a launcher focuses its instance
    /// anchor, else its first instance, else itself; a project is focused as a slot (ADR 0018).
    pub(super) fn slot_focus_target(&self, target: DesktopTarget) -> DesktopTarget {
        match target {
            DesktopTarget::Launcher(launcher) => self.focus_target_for_launcher(launcher),
            target => target,
        }
    }

    /// The leaf a project is entered down to: its focus slots followed through nested projects,
    /// else its first launcher depth first, else the project itself.
    pub(super) fn focus_target_for_project(&self, project: ProjectId) -> DesktopTarget {
        if let Some(content) = self.last_focused_content(project) {
            return self.focus_target_for_slot(content);
        }

        if let Some(launcher) = self.first_launcher_depth_first(project) {
            return self.focus_target_for_launcher(launcher);
        }

        DesktopTarget::Project(project)
    }

    pub fn last_focused_content(&self, project: ProjectId) -> Option<SlotContent> {
        let placement = self
            .aggregates
            .projects
            .get(&project)?
            .last_focused_placement?;
        self.aggregates.configuration.content_at(project, placement)
    }

    fn focus_target_for_slot(&self, content: SlotContent) -> DesktopTarget {
        match content {
            SlotContent::Launcher(launcher) => self.focus_target_for_launcher(launcher),
            SlotContent::Project(project) => self.focus_target_for_project(project),
        }
    }

    fn focus_target_for_launcher(&self, launcher: LaunchProfileId) -> DesktopTarget {
        let hierarchy = &self.aggregates.hierarchy;
        let target = self
            .aggregates
            .launchers
            .get(&launcher)
            .and_then(|presenter| presenter.focus_anchor_instance)
            .filter(|instance| {
                hierarchy.parent(&DesktopTarget::Instance(*instance))
                    == Some(&DesktopTarget::Launcher(launcher))
            })
            .or_else(|| hierarchy.launcher_instances(launcher).next())
            .map(DesktopTarget::Instance)
            .unwrap_or(DesktopTarget::Launcher(launcher));
        hierarchy.resolve_keyboard_focus_target(&target)
    }

    fn first_launcher_depth_first(&self, project: ProjectId) -> Option<LaunchProfileId> {
        for (_, content) in self.aggregates.configuration.slots_ordered(project) {
            match content {
                SlotContent::Launcher(launcher) => return Some(launcher),
                SlotContent::Project(nested) => {
                    if let Some(launcher) = self.first_launcher_depth_first(nested) {
                        return Some(launcher);
                    }
                }
            }
        }
        None
    }

    /// The focus replacement when slot `content` is removed: the first available
    /// right, left, down, or up neighbor in its matrix, else its hosting project.
    pub(super) fn slot_removal_focus(&self, content: SlotContent) -> DesktopTarget {
        let (parent, placement) = self
            .aggregates
            .configuration
            .slot_of_content(content)
            .expect("removed content is assigned to a project slot");
        let matrix_navigation =
            MatrixNavigation::new(&self.aggregates.hierarchy, &self.aggregates.configuration);
        [
            Direction::Right,
            Direction::Left,
            Direction::Down,
            Direction::Up,
        ]
        .into_iter()
        .find_map(|direction| {
            matrix_navigation.navigate_within_matrix(parent, placement, direction, None)
        })
        .map(|replacement| self.slot_focus_target(replacement))
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
                // The fullscreen camera applies from the instance's first commit,
                // view-less included: the instance target exists between
                // StartInstance and the view's first submission, and framing that
                // commit at the panel distance dollies the camera out — then back
                // in when the view arrives — the `Cmd+T` zoom-out bounce.
                if self.aggregates.instance_full_screen_mode(*instance_id)
                    == FullScreenMode::FullScreen
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
    let origin = resolve_navigation_origin(hierarchy, configuration, from)?;
    let origin_placement = navigation_origin_placement(configuration, origin);
    let column_affinity = navigation_control.plan_column_affinity(direction, origin_placement);
    let matrix_navigation = MatrixNavigation::new(hierarchy, configuration);
    let target = navigate_from_origin(matrix_navigation, origin, direction, column_affinity)?;
    let candidate =
        resolve_navigation_focus_target(hierarchy, launchers, configuration, target, direction);
    Some(NavigationPlan {
        candidate,
        column_affinity,
    })
}

fn resolve_navigation_origin(
    hierarchy: &DesktopTopology,
    configuration: &RuntimeConfiguration,
    from: &DesktopTarget,
) -> Option<NavigationOrigin> {
    match from {
        DesktopTarget::Launcher(launcher_id) => {
            let (project, placement) = configuration.slot_of_content(*launcher_id)?;
            Some(NavigationOrigin::MatrixSlot { project, placement })
        }
        DesktopTarget::Project(project) | DesktopTarget::ProjectHeader(project) => {
            let (parent, placement) = configuration.slot_of_content(*project)?;
            Some(NavigationOrigin::MatrixSlot {
                project: parent,
                placement,
            })
        }
        DesktopTarget::Instance(instance_id) => {
            let launcher = hierarchy.launcher_of_instance(*instance_id);
            let index = hierarchy
                .launcher_instances(launcher)
                .position(|instance| instance == *instance_id)?;
            Some(NavigationOrigin::Child { launcher, index })
        }
        DesktopTarget::View(_) => {
            let instance = hierarchy.instance_of_target(from)?;
            resolve_navigation_origin(hierarchy, configuration, &DesktopTarget::Instance(instance))
        }
        _ => None,
    }
}

fn navigation_origin_placement(
    configuration: &RuntimeConfiguration,
    origin: NavigationOrigin,
) -> Option<MatrixPlacement> {
    match origin {
        NavigationOrigin::MatrixSlot { placement, .. } => Some(placement),
        NavigationOrigin::Child { launcher, .. } => configuration
            .slot_of_content(launcher)
            .map(|(_, placement)| placement),
    }
}

fn navigate_from_origin(
    matrix_navigation: MatrixNavigation<'_>,
    origin: NavigationOrigin,
    direction: Direction,
    preferred_column: Option<u32>,
) -> Option<DesktopTarget> {
    match origin {
        NavigationOrigin::MatrixSlot { project, placement } => matrix_navigation
            .navigate_from_matrix_slot(project, placement, direction, preferred_column),
        NavigationOrigin::Child { launcher, index } => {
            matrix_navigation.navigate_from_child(launcher, index, direction, preferred_column)
        }
    }
}

/// Normalizes a raw navigation result into a concrete, focusable target.
///
/// Matrix navigation may return a `Launcher` shell. This step converts launcher
/// targets into concrete child instances when appropriate, then delegates to the
/// hierarchy to resolve the final focus target (for example, a nested view). A project
/// target stays the project (ADR 0018). This is the directional variant of
/// `DesktopSystem::slot_focus_target`: a band launcher picks its edge instance by direction.
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

    topology.resolve_keyboard_focus_target(&target)
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
