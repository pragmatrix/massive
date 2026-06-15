use super::effects::DesktopEffect;
use super::{DesktopSystem, DesktopTarget, Effects, OverviewTarget, UserState};
use massive_geometry::{PixelCamera, Rect, RectPx, SizePx};
use massive_scene::{ToCamera, Transform};

#[derive(Debug, Clone)]
struct OverviewBounds {
    rect: Rect,
    points: Vec<massive_geometry::Vector3>,
}

impl OverviewBounds {
    fn joined(mut self, mut other: Self) -> Self {
        self.rect = self.rect.joined(other.rect);
        self.points.append(&mut other.points);
        self
    }
}

impl DesktopSystem {
    pub(super) fn apply_zoom_in_command(&mut self) -> Effects {
        let Some(next_state) = self.next_zoom_in_state() else {
            return Effects::None;
        };

        let changed = self.user_state != next_state;
        self.user_state = next_state;

        if changed {
            DesktopEffect::UpdateCamera.into()
        } else {
            Effects::None
        }
    }

    pub(super) fn apply_zoom_out_command(&mut self) -> Effects {
        let Some(zoom_target) = self.next_zoom_out_target() else {
            return Effects::None;
        };

        let changed = !matches!(
            self.user_state,
            UserState::Overview(ref current) if *current == zoom_target
        );

        self.user_state = UserState::Overview(zoom_target);

        if changed {
            DesktopEffect::UpdateCamera.into()
        } else {
            Effects::None
        }
    }

    pub(super) fn next_zoom_out_target(&self) -> Option<OverviewTarget> {
        match &self.user_state {
            UserState::Focused => self.first_overview_target_from_focus(),
            UserState::Overview(target) => Some(self.next_overview_target(target)),
        }
    }

    fn next_zoom_in_state(&self) -> Option<UserState> {
        match &self.user_state {
            UserState::Focused => None,
            UserState::Overview(target) => Some(self.next_inward_user_state(target)),
        }
    }

    pub(super) fn overview_navigation_anchor(
        &self,
        target: &OverviewTarget,
    ) -> Option<DesktopTarget> {
        match target {
            OverviewTarget::Visor(launcher_id) | OverviewTarget::Band(launcher_id) => {
                Some(DesktopTarget::Launcher(*launcher_id))
            }
            OverviewTarget::Project(project_id) => Some(DesktopTarget::Project(*project_id)),
            OverviewTarget::Desktop => Some(DesktopTarget::Desktop),
        }
    }

    pub(super) fn overview_target_for_navigation_candidate(
        &self,
        current: &OverviewTarget,
        candidate: &DesktopTarget,
    ) -> Option<OverviewTarget> {
        match current {
            OverviewTarget::Visor(current_launcher) => {
                let candidate_launcher = self.launcher_from_target(candidate)?;
                (candidate_launcher == *current_launcher)
                    .then_some(OverviewTarget::Visor(*current_launcher))
            }
            OverviewTarget::Band(current_launcher) => {
                let current_project = self.launcher_project(*current_launcher)?;
                let candidate_launcher = self.launcher_from_target(candidate)?;
                let candidate_project = self.launcher_project(candidate_launcher)?;

                (candidate_project == current_project)
                    .then_some(OverviewTarget::Band(candidate_launcher))
            }
            OverviewTarget::Project(current_project) => {
                let candidate_project = self.project_from_target(candidate)?;
                (candidate_project == *current_project)
                    .then_some(OverviewTarget::Project(*current_project))
            }
            OverviewTarget::Desktop => Some(OverviewTarget::Desktop),
        }
    }

    pub(super) fn camera_for_overview_target(
        &self,
        target: &OverviewTarget,
    ) -> Option<PixelCamera> {
        match target {
            OverviewTarget::Visor(launcher_id) => {
                self.camera_for_bounds(self.visor_bounds(*launcher_id)?)
            }
            OverviewTarget::Band(launcher_id) => {
                self.camera_for_rect(self.band_rect(*launcher_id)?)
            }
            OverviewTarget::Project(project_id) => {
                self.camera_for_rect(self.project_rect(*project_id)?)
            }
            OverviewTarget::Desktop => self.camera_for_focus(&DesktopTarget::Desktop),
        }
    }

    fn first_overview_target_from_focus(&self) -> Option<OverviewTarget> {
        let focused = self.event_router.focused()?;

        if let Some(launcher_id) = self.launcher_from_target(focused) {
            return Some(self.first_overview_target_for_launcher(launcher_id));
        }

        if let Some(project_id) = self.project_from_target(focused) {
            return Some(OverviewTarget::Project(project_id));
        }

        Some(OverviewTarget::Desktop)
    }

    fn first_overview_target_for_launcher(
        &self,
        launcher_id: crate::projects::LaunchProfileId,
    ) -> OverviewTarget {
        if self.should_include_visor_level(launcher_id) {
            return OverviewTarget::Visor(launcher_id);
        }

        if self.should_include_band_level(launcher_id) {
            return OverviewTarget::Band(launcher_id);
        }

        self.launcher_project(launcher_id)
            .map(OverviewTarget::Project)
            .unwrap_or(OverviewTarget::Desktop)
    }

    fn deepest_overview_target_for_launcher(
        &self,
        launcher_id: crate::projects::LaunchProfileId,
    ) -> Option<OverviewTarget> {
        if self.should_include_band_level(launcher_id) {
            return Some(OverviewTarget::Band(launcher_id));
        }

        if self.should_include_visor_level(launcher_id) {
            return Some(OverviewTarget::Visor(launcher_id));
        }

        None
    }

    fn next_inward_user_state(&self, current: &OverviewTarget) -> UserState {
        match current {
            OverviewTarget::Desktop => self
                .focused_project_id()
                .map(OverviewTarget::Project)
                .map(UserState::Overview)
                .unwrap_or(UserState::Focused),
            OverviewTarget::Project(project_id) => {
                let Some(launcher_id) = self.zoom_context_launcher_for_project(*project_id) else {
                    return UserState::Focused;
                };

                self.deepest_overview_target_for_launcher(launcher_id)
                    .map(UserState::Overview)
                    .unwrap_or(UserState::Focused)
            }
            OverviewTarget::Band(launcher_id) => {
                if self.should_include_visor_level(*launcher_id) {
                    UserState::Overview(OverviewTarget::Visor(*launcher_id))
                } else {
                    UserState::Focused
                }
            }
            OverviewTarget::Visor(_) => UserState::Focused,
        }
    }

    fn next_overview_target(&self, current: &OverviewTarget) -> OverviewTarget {
        match current {
            OverviewTarget::Visor(launcher_id) => {
                if self.should_include_band_level(*launcher_id) {
                    OverviewTarget::Band(*launcher_id)
                } else {
                    self.zoom_transition_project(*launcher_id)
                        .map(OverviewTarget::Project)
                        .unwrap_or(OverviewTarget::Desktop)
                }
            }
            OverviewTarget::Band(launcher_id) => self
                .zoom_transition_project(*launcher_id)
                .map(OverviewTarget::Project)
                .unwrap_or(OverviewTarget::Desktop),
            OverviewTarget::Project(_) | OverviewTarget::Desktop => OverviewTarget::Desktop,
        }
    }

    fn zoom_transition_project(
        &self,
        launcher_id: crate::projects::LaunchProfileId,
    ) -> Option<crate::projects::ProjectId> {
        self.focused_project_id()
            .or_else(|| self.launcher_project(launcher_id))
    }

    fn should_include_visor_level(&self, launcher_id: crate::projects::LaunchProfileId) -> bool {
        self.aggregates
            .hierarchy
            .get_nested(&DesktopTarget::Launcher(launcher_id))
            .len()
            > 1
    }

    fn should_include_band_level(&self, launcher_id: crate::projects::LaunchProfileId) -> bool {
        let Some(project_id) = self.launcher_project(launcher_id) else {
            return false;
        };

        self.project_row_count(project_id) > 1
    }

    fn launcher_project(
        &self,
        launcher_id: crate::projects::LaunchProfileId,
    ) -> Option<crate::projects::ProjectId> {
        let target = DesktopTarget::Launcher(launcher_id);
        match self.aggregates.hierarchy.parent(&target) {
            Some(DesktopTarget::ProjectMatrix(project_id)) => Some(*project_id),
            _ => None,
        }
    }

    fn launcher_from_target(
        &self,
        target: &DesktopTarget,
    ) -> Option<crate::projects::LaunchProfileId> {
        match target {
            DesktopTarget::Launcher(launcher_id) => Some(*launcher_id),
            DesktopTarget::Instance(instance_id) => self.instance_launcher(*instance_id),
            DesktopTarget::View(view_id) => {
                let parent = self
                    .aggregates
                    .hierarchy
                    .parent(&DesktopTarget::View(*view_id))?;
                self.launcher_from_target(parent)
            }
            _ => None,
        }
    }

    fn project_from_target(&self, target: &DesktopTarget) -> Option<crate::projects::ProjectId> {
        match target {
            DesktopTarget::Project(project_id)
            | DesktopTarget::ProjectHeader(project_id)
            | DesktopTarget::ProjectMatrix(project_id) => Some(*project_id),
            DesktopTarget::Launcher(launcher_id) => self.launcher_project(*launcher_id),
            DesktopTarget::Instance(instance_id) => self
                .instance_launcher(*instance_id)
                .and_then(|launcher_id| self.launcher_project(launcher_id)),
            DesktopTarget::View(view_id) => {
                let parent = self
                    .aggregates
                    .hierarchy
                    .parent(&DesktopTarget::View(*view_id))?;
                self.project_from_target(parent)
            }
            DesktopTarget::Desktop => None,
        }
    }

    fn focused_project_id(&self) -> Option<crate::projects::ProjectId> {
        let focused = self.event_router.focused()?;
        self.project_from_target(focused)
    }

    fn focused_launcher_id(&self) -> Option<crate::projects::LaunchProfileId> {
        let focused = self.event_router.focused()?;
        self.launcher_from_target(focused)
    }

    fn zoom_context_launcher_for_project(
        &self,
        project_id: crate::projects::ProjectId,
    ) -> Option<crate::projects::LaunchProfileId> {
        if let Some(focused_launcher) = self.focused_launcher_id()
            && self.launcher_project(focused_launcher) == Some(project_id)
        {
            return Some(focused_launcher);
        }

        self.aggregates
            .hierarchy
            .get_nested(&DesktopTarget::ProjectMatrix(project_id))
            .iter()
            .find_map(|target| match target {
                DesktopTarget::Launcher(launcher_id) => Some(*launcher_id),
                _ => None,
            })
    }

    fn project_row_count(&self, project_id: crate::projects::ProjectId) -> usize {
        let mut rows = std::collections::HashSet::new();

        for target in self
            .aggregates
            .hierarchy
            .get_nested(&DesktopTarget::ProjectMatrix(project_id))
        {
            let DesktopTarget::Launcher(launcher_id) = target else {
                continue;
            };

            let Some(launcher) = self.aggregates.launchers.get(launcher_id) else {
                continue;
            };

            rows.insert(launcher.placement.row);
        }

        rows.len()
    }

    fn visor_bounds(
        &self,
        launcher_id: crate::projects::LaunchProfileId,
    ) -> Option<OverviewBounds> {
        let root = DesktopTarget::Launcher(launcher_id);
        let mut bounds = self.target_bounds(&root);
        self.extend_bounds_with_subtree(&root, &mut bounds);
        bounds
    }

    fn band_rect(&self, launcher_id: crate::projects::LaunchProfileId) -> Option<Rect> {
        let project_id = self.launcher_project(launcher_id)?;
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

            let Some(launcher_rect) =
                self.target_rect(&DesktopTarget::Launcher(*candidate_launcher))
            else {
                continue;
            };

            rect = Some(match rect {
                Some(existing) => existing.joined(launcher_rect),
                None => launcher_rect,
            });
        }

        rect
    }

    fn project_rect(&self, project_id: crate::projects::ProjectId) -> Option<Rect> {
        let root = DesktopTarget::Project(project_id);
        let mut rect = self.target_rect(&root);
        self.extend_rect_with_subtree(&root, &mut rect);
        rect
    }

    fn extend_rect_with_subtree(&self, root: &DesktopTarget, rect: &mut Option<Rect>) {
        for child in self.aggregates.hierarchy.get_nested(root) {
            if let Some(child_rect) = self.target_rect(child) {
                *rect = Some(match *rect {
                    Some(existing) => existing.joined(child_rect),
                    None => child_rect,
                });
            }

            self.extend_rect_with_subtree(child, rect);
        }
    }

    fn extend_bounds_with_subtree(
        &self,
        root: &DesktopTarget,
        bounds: &mut Option<OverviewBounds>,
    ) {
        for child in self.aggregates.hierarchy.get_nested(root) {
            if let Some(child_bounds) = self.target_bounds(child) {
                *bounds = Some(match bounds.take() {
                    Some(existing) => existing.joined(child_bounds),
                    None => child_bounds,
                });
            }

            self.extend_bounds_with_subtree(child, bounds);
        }
    }

    fn target_bounds(&self, target: &DesktopTarget) -> Option<OverviewBounds> {
        let placement = self.placement(target)?;
        let rect_px: RectPx = placement.rect.into();
        let size = Rect::from(rect_px).size();
        // `placement.transform` is anchor-space for this target. Convert to origin-space
        // before transforming local rectangle corners.
        let local_rect = size.to_rect();
        let local_center = local_rect.center();
        let origin_transform = placement.transform.to_origin_space(local_center);
        Some(Self::transform_rect(local_rect, origin_transform))
    }

    fn target_rect(&self, target: &DesktopTarget) -> Option<Rect> {
        let placement = self.placement(target)?;
        let rect_px: RectPx = placement.rect.into();
        let size = Rect::from(rect_px).size();
        let local_rect = size.to_rect();
        let local_center = local_rect.center();
        let origin_transform = placement.transform.to_origin_space(local_center);
        let bounds = Self::transform_rect(local_rect, origin_transform);
        Some(bounds.rect)
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
        center: massive_geometry::Vector3,
        points: &[massive_geometry::Vector3],
        fovy: f64,
        surface_size: SizePx,
    ) -> massive_geometry::Size {
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
        target_size: massive_geometry::Size,
        center: massive_geometry::Vector3,
        points: &[massive_geometry::Vector3],
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
