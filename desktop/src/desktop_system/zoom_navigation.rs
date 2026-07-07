use massive_geometry::{PixelCamera, Rect, RectPx, Size, SizePx, Vector3};
use massive_layout::LayoutTopology;
use massive_scene::{ToCamera, Transform};

use crate::desktop_system::LauncherMap;
use crate::desktop_system::topology::DesktopTopology;
use crate::projects::{LaunchProfileId, ProjectId};

use super::{DesktopSystem, DesktopTarget, OverviewTarget, UserState};

#[derive(Debug, Clone)]
struct OverviewBounds {
    rect: Rect,
    points: Vec<Vector3>,
}

impl OverviewBounds {
    fn joined(mut self, mut other: Self) -> Self {
        self.rect = self.rect.joined(other.rect);
        self.points.append(&mut other.points);
        self
    }
}

#[must_use]
pub fn zoom_in(
    topo: &DesktopTopology,
    launchers: &LauncherMap,
    focused: DesktopTarget,
    user_state: UserState,
) -> UserState {
    {
        match &user_state {
            UserState::Focused => None,
            UserState::Overview(target) => {
                Some(next_inward_user_state(topo, launchers, focused, target))
            }
        }
    }
    .unwrap_or(user_state)
}

#[must_use]
pub fn zoom_out(
    topology: &DesktopTopology,
    launchers: &LauncherMap,
    focused: DesktopTarget,
    user_state: UserState,
) -> UserState {
    {
        match &user_state {
            UserState::Focused => first_overview_target_from_focus(topology, launchers, focused),
            UserState::Overview(target) => Some(next_outward_overview_target(
                topology, launchers, focused, target,
            )),
        }
    }
    .map(UserState::Overview)
    .unwrap_or(user_state)
}

pub fn overview_target_for_navigation_candidate(
    topology: &DesktopTopology,
    current: &OverviewTarget,
    candidate: &DesktopTarget,
) -> Option<OverviewTarget> {
    match current {
        OverviewTarget::Visor(current_launcher) => {
            let candidate_launcher = topology.launcher_of_target(candidate)?;
            (candidate_launcher == *current_launcher)
                .then_some(OverviewTarget::Visor(*current_launcher))
        }
        OverviewTarget::MatrixRow(current_launcher) => {
            let current_project = topology.project_of_launcher(*current_launcher)?;
            let candidate_launcher = topology.launcher_of_target(candidate)?;
            let candidate_project = topology.project_of_launcher(candidate_launcher)?;

            (candidate_project == current_project)
                .then_some(OverviewTarget::MatrixRow(candidate_launcher))
        }
        OverviewTarget::Project(current_project) => {
            let candidate_project = topology.project_of_target(candidate)?;
            (candidate_project == *current_project)
                .then_some(OverviewTarget::Project(*current_project))
        }
        OverviewTarget::Desktop => Some(OverviewTarget::Desktop),
    }
}

impl DesktopSystem {
    pub(super) fn camera_for_overview_target(
        &self,
        target: &OverviewTarget,
    ) -> Option<PixelCamera> {
        match target {
            OverviewTarget::Visor(launcher_id) => {
                self.camera_for_bounds(self.visor_bounds(*launcher_id))
            }
            OverviewTarget::MatrixRow(launcher_id) => {
                self.camera_for_rect(self.matrix_row_rect(*launcher_id)?)
            }
            OverviewTarget::Project(project_id) => {
                self.camera_for_rect(self.project_rect(*project_id))
            }
            OverviewTarget::Desktop => self.camera_for_focus(&DesktopTarget::Desktop),
        }
    }
}

fn first_overview_target_from_focus(
    topology: &DesktopTopology,
    launchers: &LauncherMap,
    focused: DesktopTarget,
) -> Option<OverviewTarget> {
    if let Some(launcher_id) = topology.launcher_of_target(&focused) {
        return Some(first_overview_target_for_launcher(
            topology,
            launchers,
            launcher_id,
        ));
    }

    if let Some(project_id) = topology.project_of_target(&focused) {
        return Some(OverviewTarget::Project(project_id));
    }

    Some(OverviewTarget::Desktop)
}

fn first_overview_target_for_launcher(
    topology: &DesktopTopology,
    launchers: &LauncherMap,
    launcher_id: LaunchProfileId,
) -> OverviewTarget {
    if should_include_visor_level(topology, launcher_id) {
        return OverviewTarget::Visor(launcher_id);
    }

    if should_include_matrix_row_level(topology, launchers, launcher_id) {
        return OverviewTarget::MatrixRow(launcher_id);
    }

    topology
        .project_of_launcher(launcher_id)
        .map(OverviewTarget::Project)
        .unwrap_or(OverviewTarget::Desktop)
}

fn next_inward_user_state(
    topology: &DesktopTopology,
    launchers: &LauncherMap,
    focused: DesktopTarget,
    current: &OverviewTarget,
) -> UserState {
    match current {
        OverviewTarget::Desktop => topology
            .project_of_target(&focused)
            .map(OverviewTarget::Project)
            .map(UserState::Overview)
            .unwrap_or(UserState::Focused),
        OverviewTarget::Project(project_id) => {
            let Some(launcher_id) =
                zoom_context_launcher_for_project(topology, focused, *project_id)
            else {
                return UserState::Focused;
            };

            deepest_overview_target_for_launcher(topology, launchers, launcher_id)
                .map(UserState::Overview)
                .unwrap_or(UserState::Focused)
        }
        OverviewTarget::MatrixRow(launcher_id) => {
            if should_include_visor_level(topology, *launcher_id) {
                UserState::Overview(OverviewTarget::Visor(*launcher_id))
            } else {
                UserState::Focused
            }
        }
        OverviewTarget::Visor(_) => UserState::Focused,
    }
}

fn deepest_overview_target_for_launcher(
    topology: &DesktopTopology,
    launchers: &LauncherMap,
    launcher_id: LaunchProfileId,
) -> Option<OverviewTarget> {
    if should_include_matrix_row_level(topology, launchers, launcher_id) {
        return Some(OverviewTarget::MatrixRow(launcher_id));
    }

    if should_include_visor_level(topology, launcher_id) {
        return Some(OverviewTarget::Visor(launcher_id));
    }

    None
}

fn next_outward_overview_target(
    topology: &DesktopTopology,
    launchers: &LauncherMap,
    focused: DesktopTarget,
    current: &OverviewTarget,
) -> OverviewTarget {
    match current {
        OverviewTarget::Visor(launcher_id) => {
            if should_include_matrix_row_level(topology, launchers, *launcher_id) {
                OverviewTarget::MatrixRow(*launcher_id)
            } else {
                zoom_transition_project(topology, focused, *launcher_id)
                    .map(OverviewTarget::Project)
                    .unwrap_or(OverviewTarget::Desktop)
            }
        }
        OverviewTarget::MatrixRow(launcher_id) => {
            zoom_transition_project(topology, focused, *launcher_id)
                .map(OverviewTarget::Project)
                .unwrap_or(OverviewTarget::Desktop)
        }
        OverviewTarget::Project(_) | OverviewTarget::Desktop => OverviewTarget::Desktop,
    }
}

fn zoom_transition_project(
    topology: &DesktopTopology,
    focused: DesktopTarget,
    launcher_id: LaunchProfileId,
) -> Option<ProjectId> {
    topology
        .project_of_target(&focused)
        .or_else(|| topology.project_of_launcher(launcher_id))
}

fn should_include_visor_level(topology: &DesktopTopology, launcher_id: LaunchProfileId) -> bool {
    topology
        .children_of(&DesktopTarget::Launcher(launcher_id))
        .iter()
        .filter(|target| matches!(target, DesktopTarget::Instance(_)))
        .count()
        > 1
}

fn should_include_matrix_row_level(
    topology: &DesktopTopology,
    launchers: &LauncherMap,
    launcher_id: LaunchProfileId,
) -> bool {
    let Some(project_id) = topology.project_of_launcher(launcher_id) else {
        return false;
    };

    let Some(row) = launchers.get(&launcher_id).map(|l| l.placement.row) else {
        return false;
    };

    let launcher_count = topology
        .children_of(&DesktopTarget::ProjectMatrix(project_id))
        .iter()
        .filter_map(|target| match target {
            DesktopTarget::Launcher(candidate_launcher) => launchers.get(candidate_launcher),
            _ => None,
        })
        .filter(|launcher| launcher.placement.row == row)
        .count();

    launcher_count > 1
}

fn zoom_context_launcher_for_project(
    topology: &DesktopTopology,
    focused: DesktopTarget,
    project_id: ProjectId,
) -> Option<crate::projects::LaunchProfileId> {
    // Simplify: Can't we use project_of_target directly here?
    if let Some(focused_launcher) = topology.launcher_of_target(&focused)
        && topology.project_of_launcher(focused_launcher) == Some(project_id)
    {
        return Some(focused_launcher);
    }

    None
}

impl DesktopSystem {
    fn visor_bounds(&self, launcher_id: crate::projects::LaunchProfileId) -> OverviewBounds {
        let root = DesktopTarget::Launcher(launcher_id);
        let mut bounds = Some(self.target_bounds(&root));
        self.extend_bounds_with_subtree(&root, &mut bounds);
        bounds.expect("Internal error: launcher bounds should always exist")
    }

    fn matrix_row_rect(&self, launcher_id: crate::projects::LaunchProfileId) -> Option<Rect> {
        let project_id = self.aggregates.hierarchy.project_of_launcher(launcher_id)?;
        let row = self.aggregates.launchers.get(&launcher_id)?.placement.row;
        let mut rect: Option<Rect> = None;

        for target in self
            .aggregates
            .hierarchy
            .get_nested(&DesktopTarget::ProjectMatrix(project_id))
        {
            let DesktopTarget::Launcher(candidate_launcher) = target else {
                continue;
            };

            let Some(candidate) = self.aggregates.launchers.get(candidate_launcher) else {
                continue;
            };

            if candidate.placement.row != row {
                continue;
            }

            let launcher_rect = self.target_rect(&DesktopTarget::Launcher(*candidate_launcher));

            rect = Some(match rect {
                Some(existing) => existing.joined(launcher_rect),
                None => launcher_rect,
            });
        }

        // Be sure the full width of the project is covered.
        rect.map(|matrix_row_rect| {
            let project_matrix_rect = self.project_matrix_rect(project_id);
            (
                project_matrix_rect.left,
                matrix_row_rect.top,
                project_matrix_rect.right,
                matrix_row_rect.bottom,
            )
                .into()
        })
    }

    fn project_matrix_rect(&self, project_id: crate::projects::ProjectId) -> Rect {
        self.target_rect(&DesktopTarget::ProjectMatrix(project_id))
    }

    fn project_rect(&self, project_id: crate::projects::ProjectId) -> Rect {
        let root = DesktopTarget::Project(project_id);
        let mut rect = Some(self.target_rect(&root));
        self.extend_rect_with_subtree(&root, &mut rect);
        rect.expect("Internal error: project bounds should always exist")
    }

    fn extend_rect_with_subtree(&self, root: &DesktopTarget, rect: &mut Option<Rect>) {
        for child in self.aggregates.hierarchy.get_nested(root) {
            let child_rect = self.target_rect(child);
            *rect = Some(match *rect {
                Some(existing) => existing.joined(child_rect),
                None => child_rect,
            });

            self.extend_rect_with_subtree(child, rect);
        }
    }

    fn extend_bounds_with_subtree(
        &self,
        root: &DesktopTarget,
        bounds: &mut Option<OverviewBounds>,
    ) {
        for child in self.aggregates.hierarchy.get_nested(root) {
            let child_bounds = self.target_bounds(child);
            *bounds = Some(match bounds.take() {
                Some(existing) => existing.joined(child_bounds),
                None => child_bounds,
            });

            self.extend_bounds_with_subtree(child, bounds);
        }
    }

    fn target_bounds(&self, target: &DesktopTarget) -> OverviewBounds {
        let placement = self.placement(target);
        let rect_px: RectPx = placement.rect.into();
        let size = Rect::from(rect_px).size();
        // `placement.transform` is anchor-space for this target. Convert to origin-space
        // before transforming local rectangle corners.
        let local_rect = size.to_rect();
        let local_center = local_rect.center();
        let origin_transform = placement.transform.to_origin_space(local_center);
        Self::transform_rect(local_rect, origin_transform)
    }

    fn target_rect(&self, target: &DesktopTarget) -> Rect {
        let placement = self.placement(target);
        let rect_px: RectPx = placement.rect.into();
        let size = Rect::from(rect_px).size();
        let local_rect = size.to_rect();
        let local_center = local_rect.center();
        let origin_transform = placement.transform.to_origin_space(local_center);
        let bounds = Self::transform_rect(local_rect, origin_transform);
        bounds.rect
    }

    fn camera_for_bounds(&self, bounds: OverviewBounds) -> Option<PixelCamera> {
        if bounds.rect.is_empty() {
            return None;
        }

        let center = bounds.rect.center();
        let center: Transform = (center.x, center.y, 0.0).into();
        let camera = center.to_camera();
        let surface_size = self.window.inner_size();
        let target_size = Self::fit_size_for_points(
            bounds.rect,
            center.translate,
            &bounds.points,
            camera.fovy,
            surface_size,
        );
        Some(camera.with_size(target_size))
    }

    fn camera_for_rect(&self, rect: Rect) -> Option<PixelCamera> {
        if rect.is_empty() {
            return None;
        }

        let center = rect.center();
        let center: Transform = (center.x, center.y, 0.0).into();
        Some(center.to_camera().with_size(rect.size()))
    }

    fn fit_size_for_points(
        rect: Rect,
        center: Vector3,
        points: &[Vector3],
        fovy: f64,
        surface_size: SizePx,
    ) -> Size {
        if points.is_empty() {
            return rect.size();
        }

        let base_size = rect.size();
        let mut low = 1.0;
        let mut high = 1.0;

        while !Self::points_fit_in_surface(base_size * high, center, points, fovy, surface_size) {
            high *= 2.0;
            if high > 1024.0 {
                return base_size * high;
            }
        }

        for _ in 0..40 {
            let mid = (low + high) * 0.5;
            if Self::points_fit_in_surface(base_size * mid, center, points, fovy, surface_size) {
                high = mid;
            } else {
                low = mid;
            }
        }

        base_size * high
    }

    fn points_fit_in_surface(
        target_size: Size,
        center: Vector3,
        points: &[Vector3],
        fovy: f64,
        surface_size: SizePx,
    ) -> bool {
        let surface_width = surface_size.width as f64;
        let surface_height = surface_size.height as f64;
        if target_size.width <= 0.0 || target_size.height <= 0.0 {
            return false;
        }

        let target_scale =
            (surface_width / target_size.width).min(surface_height / target_size.height);
        let camera_distance = 1.0 / (fovy * 0.5).to_radians().tan();
        let model_to_ndc_scale = 2.0 / surface_height;
        let z_scale = model_to_ndc_scale * target_scale;
        let half_surface_width = surface_width * 0.5;
        let half_surface_height = surface_height * 0.5;

        for point in points {
            let dx = point.x - center.x;
            let dy = point.y - center.y;
            let denominator = camera_distance - z_scale * point.z;
            if denominator <= 0.0 {
                return false;
            }

            let x = camera_distance * target_scale * dx / denominator;
            let y = camera_distance * target_scale * dy / denominator;

            if x.abs() > half_surface_width || y.abs() > half_surface_height {
                return false;
            }
        }

        true
    }

    fn transform_rect(rect: Rect, transform: Transform) -> OverviewBounds {
        let quad = rect.to_quad();

        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut points = Vec::with_capacity(4);

        for point in quad {
            let transformed = transform.transform_point((point.x, point.y, 0.0).into());
            min_x = min_x.min(transformed.x);
            min_y = min_y.min(transformed.y);
            max_x = max_x.max(transformed.x);
            max_y = max_y.max(transformed.y);
            points.push(transformed);
        }

        OverviewBounds {
            rect: (min_x, min_y, max_x, max_y).into(),
            points,
        }
    }
}
