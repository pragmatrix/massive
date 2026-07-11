use massive_geometry::{PixelCamera, Rect, RectPx, Size, SizePx, Vector3};
use massive_scene::{ToCamera, Transform};

use super::{DesktopSystem, DesktopTarget};
use crate::desktop_system::FocusDepth;
use crate::projects::LaunchProfileId;

#[derive(Debug, Clone)]
pub(super) struct OverviewBounds {
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

pub(super) fn focus_depth_from_target(target: &DesktopTarget) -> FocusDepth {
    match target {
        DesktopTarget::View(_) => FocusDepth::Instance,
        DesktopTarget::Instance(_) => FocusDepth::Instance,
        DesktopTarget::Launcher(_) => FocusDepth::Launcher,
        DesktopTarget::Project(_)
        | DesktopTarget::ProjectHeader(_)
        | DesktopTarget::ProjectMatrix(_) => FocusDepth::Project,
        DesktopTarget::Desktop => FocusDepth::Desktop,
    }
}

impl DesktopSystem {
    pub(super) fn resolve_camera_for_target_or_ancestor(
        &self,
        target: &DesktopTarget,
        mut depth: FocusDepth,
    ) -> Option<PixelCamera> {
        loop {
            if let Some(camera) = self.resolve_camera_focus_and_depth(target, depth) {
                return Some(camera);
            }

            depth = depth.zoom_out()?;
        }
    }

    pub(super) fn resolve_camera_focus_and_depth(
        &self,
        target: &DesktopTarget,
        depth: FocusDepth,
    ) -> Option<PixelCamera> {
        match depth {
            FocusDepth::Instance => self
                .aggregates
                .hierarchy
                .instance_of_target(target)
                .and_then(|_| self.camera_for_target(target)),
            FocusDepth::Launcher => self.camera_for_launcher_focus(target),
            FocusDepth::Row => self
                .aggregates
                .hierarchy
                .launcher_of_target(target)
                .and_then(|launcher| self.camera_for_rect(self.matrix_row_rect(launcher)?)),
            FocusDepth::Project => self
                .aggregates
                .hierarchy
                .project_of_target(target)
                .and_then(|project| self.camera_for_rect(self.project_rect(project))),
            FocusDepth::Desktop => self.camera_for_target(&DesktopTarget::Desktop),
        }
    }

    fn camera_for_launcher_focus(&self, target: &DesktopTarget) -> Option<PixelCamera> {
        let launcher_id = self.aggregates.hierarchy.launcher_of_target(target)?;
        let launcher = DesktopTarget::Launcher(launcher_id);

        if self
            .aggregates
            .hierarchy
            .launcher_instances(launcher_id)
            .len()
            > 1
        {
            self.camera_for_bounds(self.launcher_bounds(launcher_id))
        } else {
            self.camera_for_target(&launcher)
        }
    }

    pub(super) fn camera_for_bounds(&self, bounds: OverviewBounds) -> Option<PixelCamera> {
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

    pub(super) fn camera_for_rect(&self, rect: Rect) -> Option<PixelCamera> {
        if rect.is_empty() {
            return None;
        }

        let center = rect.center();
        let center: Transform = (center.x, center.y, 0.0).into();
        Some(center.to_camera().with_size(rect.size()))
    }

    pub(super) fn launcher_bounds(&self, launcher_id: LaunchProfileId) -> OverviewBounds {
        let root = DesktopTarget::Launcher(launcher_id);
        let mut bounds = Some(self.target_bounds(&root));
        self.extend_bounds_with_subtree(&root, &mut bounds);
        bounds.expect("Internal error: launcher bounds should always exist")
    }

    pub(super) fn matrix_row_rect(&self, launcher_id: LaunchProfileId) -> Option<Rect> {
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

        rect.map(|matrix_row_rect| self.with_desktop_width(matrix_row_rect))
    }

    pub(super) fn project_rect(&self, project_id: crate::projects::ProjectId) -> Rect {
        let root = DesktopTarget::Project(project_id);
        let mut rect = Some(self.target_rect(&root));
        self.extend_rect_with_subtree(&root, &mut rect);
        self.with_desktop_width(rect.expect("Internal error: project bounds should always exist"))
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

    fn with_desktop_width(&self, rect: Rect) -> Rect {
        let desktop_rect = self.target_rect(&DesktopTarget::Desktop);
        (desktop_rect.left, rect.top, desktop_rect.right, rect.bottom).into()
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
