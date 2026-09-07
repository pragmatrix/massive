use massive_applications::InstanceId;
use massive_geometry::{PixelCamera, Rect, RectPx, Size, SizePx, Vector3};
use massive_scene::prelude::*;

use crate::desktop_system::{DesktopSystem, DesktopTarget, FocusDepth};
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

pub(crate) fn focus_depth_from_target(target: &DesktopTarget) -> FocusDepth {
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
    pub(crate) fn resolve_camera_for_target_or_ancestor(
        &self,
        target: &DesktopTarget,
        mut depth: FocusDepth,
        window_size: SizePx,
    ) -> PixelCamera {
        loop {
            if let Some(camera) = self.resolve_camera_focus_and_depth(target, depth, window_size) {
                return camera;
            }

            depth = depth
                .zoom_out()
                .expect("Internal error: no camera found for target or ancestor");
        }
    }

    fn resolve_camera_focus_and_depth(
        &self,
        target: &DesktopTarget,
        depth: FocusDepth,
        window_size: SizePx,
    ) -> Option<PixelCamera> {
        match depth {
            FocusDepth::InstanceFullScreen => self
                .aggregates
                .hierarchy
                .instance_of_target(target)
                .map(|instance_id| {
                    let presentation = self.resolve_instance_presentation(instance_id, window_size);
                    let transform = self
                        .placement(&DesktopTarget::Instance(instance_id))
                        .transform;
                    let scale = Self::fit_scale(presentation.layout_size(), window_size);
                    Self::camera_from_placement(transform).with_scale(scale)
                }),
            FocusDepth::Instance => self.camera_for_target(target, window_size),
            FocusDepth::Launcher => self.camera_for_launcher_focus(target, window_size),
            FocusDepth::Row => self
                .aggregates
                .hierarchy
                .launcher_of_target(target)
                .and_then(|launcher| {
                    self.camera_for_rect(self.matrix_row_rect(launcher)?, window_size)
                }),
            FocusDepth::Project => self
                .aggregates
                .hierarchy
                .project_of_target(target)
                .and_then(|project| self.camera_for_rect(self.project_rect(project), window_size)),
            FocusDepth::Desktop => self.camera_for_target(&DesktopTarget::Desktop, window_size),
        }
    }

    fn camera_for_launcher_focus(
        &self,
        target: &DesktopTarget,
        window_size: SizePx,
    ) -> Option<PixelCamera> {
        let launcher_id = self.aggregates.hierarchy.launcher_of_target(target)?;

        let instances = self.aggregates.hierarchy.launcher_instances(launcher_id);
        if instances.is_empty() {
            // A launcher with no visors has nothing to union — frame the launcher itself.
            return self.camera_for_target(&DesktopTarget::Launcher(launcher_id), window_size);
        }

        let bounds = self.fold_instance_bounds(instances);
        self.camera_for_bounds(bounds, window_size)
    }

    fn camera_for_bounds(
        &self,
        bounds: OverviewBounds,
        window_size: SizePx,
    ) -> Option<PixelCamera> {
        if bounds.rect.is_empty() {
            return None;
        }

        // Center the whole-set fit on the axis-aligned union center (identity rotation, z=0) so
        // the overview hugs the actual visor extent instead of inheriting the focused panel's
        // offset position/rotation.
        let center = bounds.rect.center();
        let look_at: Transform = (center.x, center.y, 0.0).into();
        let camera = look_at.to_camera();
        let target_size = Self::fit_size_for_points(
            bounds.rect,
            look_at,
            &bounds.points,
            camera.fovy,
            window_size,
        );
        let scale = Self::fit_scale(target_size, window_size);
        Some(camera.with_scale(scale))
    }

    fn camera_for_rect(&self, rect: Rect, window_size: SizePx) -> Option<PixelCamera> {
        if rect.is_empty() {
            return None;
        }

        let center = rect.center();
        let center: Transform = (center.x, center.y, 0.0).into();
        let scale = Self::fit_scale(rect.size(), window_size);
        Some(center.to_camera().with_scale(scale))
    }

    // Frame only the visor instances, excluding the launcher's own background rect so the
    // overview doesn't span further left/right than the visible visors.
    fn fold_instance_bounds(&self, instances: Vec<InstanceId>) -> OverviewBounds {
        let mut bounds: Option<OverviewBounds> = None;
        for instance in instances {
            let instance_bounds = self.target_bounds(&DesktopTarget::Instance(instance));
            bounds = Some(match bounds {
                Some(existing) => existing.joined(instance_bounds),
                None => instance_bounds,
            });
        }
        bounds.expect("Internal error: a launcher with visors must yield bounds")
    }

    pub(super) fn matrix_row_rect(&self, launcher_id: LaunchProfileId) -> Option<Rect> {
        let project_id = self.aggregates.hierarchy.project_of_launcher(launcher_id);
        let row = self.aggregates.matrix_positions.get(&launcher_id)?.row;
        let mut rect: Option<Rect> = None;

        for candidate_launcher in self.aggregates.hierarchy.matrix_launchers(project_id) {
            let Some(candidate) = self.aggregates.matrix_positions.get(&candidate_launcher) else {
                continue;
            };

            if candidate.row != row {
                continue;
            }

            let launcher_rect = self.target_rect(&DesktopTarget::Launcher(candidate_launcher));

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
        look_at: Transform,
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

        while !Self::points_fit_in_surface(base_size * high, look_at, points, fovy, surface_size) {
            high *= 2.0;
            if high > 1024.0 {
                return base_size * high;
            }
        }

        for _ in 0..40 {
            let mid = (low + high) * 0.5;
            if Self::points_fit_in_surface(base_size * mid, look_at, points, fovy, surface_size) {
                high = mid;
            } else {
                low = mid;
            }
        }

        base_size * high
    }

    fn points_fit_in_surface(
        target_size: Size,
        look_at: Transform,
        points: &[Vector3],
        fovy: f64,
        surface_size: SizePx,
    ) -> bool {
        let surface_size: Size = surface_size.into();
        if target_size.is_empty() {
            return false;
        }

        let target_scale = (surface_size / target_size).min_element();
        let camera_distance = 1.0 / (fovy * 0.5).to_radians().tan();
        let model_to_ndc_scale = 2.0 / surface_size.height;
        let z_scale = model_to_ndc_scale * target_scale;
        let half_surface = surface_size * 0.5;

        // Transform points into camera space so a rotated look-at frames correctly.
        let to_camera = look_at.inverse();

        for point in points {
            let camera_point = to_camera.transform_point(*point);
            let denominator = camera_distance - z_scale * camera_point.z;
            if denominator <= 0.0 {
                return false;
            }

            let x = camera_distance * target_scale * camera_point.x / denominator;
            let y = camera_distance * target_scale * camera_point.y / denominator;

            if x.abs() > half_surface.width || y.abs() > half_surface.height {
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
