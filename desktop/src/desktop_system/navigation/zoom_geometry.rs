//! Zoom camera geometry: resolves the camera that frames the focused target at a zoom level
//! from the placed layout, plus the project and slot lookups that decide what is framed.

use derive_more::Debug;
use massive_applications::InstanceId;
use massive_geometry::{
    BoundaryRect, Centroid, PixelCamera, Quaternion, Rect, RectPx, Size, SizePx, Vector3,
};
use massive_layout::Placement;
use massive_scene::prelude::*;

use crate::desktop_system::layout_state::DesktopLayoutState;
use crate::desktop_system::topology::DesktopTopology;
use crate::desktop_system::{DesktopTarget, ZoomLevel};
use crate::focus_path::PathResolver;
use crate::projects::{
    LauncherMode, MatrixPlacement, ProjectId, RuntimeConfiguration, SlotContent,
};

/// Resolves where the camera points for a zoom level, from the placed layout.
#[derive(Debug)]
pub struct ZoomGeometry<'a> {
    hierarchy: &'a DesktopTopology,
    configuration: &'a RuntimeConfiguration,
    #[debug(skip)]
    layout_state: &'a DesktopLayoutState,
    window_size: SizePx,
}

impl<'a> ZoomGeometry<'a> {
    pub fn new(
        hierarchy: &'a DesktopTopology,
        configuration: &'a RuntimeConfiguration,
        layout_state: &'a DesktopLayoutState,
        window_size: SizePx,
    ) -> Self {
        Self {
            hierarchy,
            configuration,
            layout_state,
            window_size,
        }
    }

    pub fn parent_project_of(&self, project: ProjectId) -> Option<ProjectId> {
        self.hierarchy.parent_project_of(project)
    }

    pub fn project_nesting_depth(&self, project: ProjectId) -> usize {
        self.hierarchy.project_nesting_depth(project)
    }

    /// The project whose matrix holds `target`'s slot. A project target is a slot of its parent;
    /// the root project, which no matrix hosts, is its own zoom project.
    pub fn project_of(&self, target: &DesktopTarget) -> ProjectId {
        match target.stands_for_project() {
            Some(project) => self.hierarchy.parent_project_of(project).unwrap_or(project),
            None => self.hierarchy.project_of_target(target),
        }
    }

    pub fn slot_in_project(
        &self,
        project: ProjectId,
        target: &DesktopTarget,
    ) -> Option<(SlotContent, MatrixPlacement)> {
        let target_path = self.hierarchy.resolve_path(Some(target));
        self.configuration
            .slots_ordered(project)
            .find_map(|(placement, content)| {
                target_path
                    .contains(&content.target())
                    .then_some((content, placement))
            })
    }

    pub fn resolve_camera_for_target(
        &self,
        target: &DesktopTarget,
        level: ZoomLevel,
    ) -> PixelCamera {
        let project = self.project_of(target);
        let Some((content, placement)) = self.slot_in_project(project, target) else {
            return self.project_camera(project);
        };

        match level.normalized(target.is_instance_target()) {
            ZoomLevel::Focus if target.is_instance_target() => self
                .camera_for_target(target)
                .expect("an instance focus target must have a camera"),
            ZoomLevel::Focus | ZoomLevel::Slot => self
                .camera_for_slot(content)
                .expect("a configured project slot must have a camera"),
            ZoomLevel::Row => {
                let rect = self
                    .matrix_row_rect_for_project(project, placement.row)
                    .expect("a row containing the focused target must have bounds");
                self.camera_for_rect(rect)
                    .expect("a row containing the focused target must have a camera")
            }
            ZoomLevel::Project => self.project_camera(project),
        }
    }

    fn project_camera(&self, project: ProjectId) -> PixelCamera {
        self.camera_for_rect(self.project_rect(project))
            .expect("a live project must have camera bounds")
    }

    fn camera_for_target(&self, focus: &DesktopTarget) -> Option<PixelCamera> {
        match focus {
            DesktopTarget::Desktop => {
                // The desktop node has no presenter to frame; its child (the root
                // project) carries the camera.
                self.camera_for_target(self.hierarchy.parent(focus)?)
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
                // The camera frames the instance at its content scale, the scale the layout gives
                // the instance's children: 1 at regular size, and in Full Screen Mode (ADR 0014)
                // the factor that fits their window-resolution content into the instance extent.
                // Dollying by it maps that content 1:1 onto the screen.
                //
                // The title bar is the source because it exists with the instance (ADR 0019), so a
                // view-less fullscreen instance already frames fullscreen and the camera does not
                // zoom out and back in when the view arrives.
                let content_scale = self
                    .placement(&DesktopTarget::InstanceTitleBar(*instance_id))
                    .transform
                    .scale;
                Some(Self::camera_from_placement(Transform::new(
                    transform.translate,
                    transform.rotate,
                    content_scale,
                )))
            }
            DesktopTarget::InstanceTitleBar(_) | DesktopTarget::View(_) => {
                self.camera_for_target(self.hierarchy.parent(focus)?)
            }
        }
    }

    fn camera_for_slot(&self, content: SlotContent) -> Option<PixelCamera> {
        match content {
            SlotContent::Launcher(launcher) => {
                self.camera_for_launcher_focus(&DesktopTarget::Launcher(launcher))
            }
            SlotContent::Project(nested) => {
                // Parent slots contain the project's full presented subtree.
                self.camera_for_rect(self.project_rect(nested))
            }
        }
    }

    fn camera_for_launcher_focus(&self, target: &DesktopTarget) -> Option<PixelCamera> {
        let launcher_id = self.hierarchy.launcher_of_target(target)?;

        let instances: Vec<_> = self.hierarchy.launcher_instances(launcher_id).collect();
        if instances.len() <= 1 {
            // A launcher with zero or one visor has no arc to union — frame the launcher itself.
            return self.camera_for_target(&DesktopTarget::Launcher(launcher_id));
        }

        // The band/visor camera split follows the configuration's mode; a launcher
        // outside the configuration falls through to the visor camera, the default mode.
        match self
            .configuration
            .launcher(launcher_id)
            .map(|launcher| launcher.mode)
        {
            // Band panels are flat axis-aligned rects (no yaw, z = 0): the simple letterbox fit
            // the rows and projects use.
            Some(LauncherMode::Band) => self.camera_for_rect(self.fold_instance_rect(instances)),
            // The arc camera is visor-specific; a launcher missing from the configuration falls
            // through to it, the default mode.
            Some(LauncherMode::Visor) | None => self.camera_for_visor_arc(instances),
        }
    }

    // Orient toward the arc's asymmetric mass: the arc is re-centered on the focused panel, so
    // an off-center focus fans the other panels to one side and their panel yaws average to a
    // nonzero angle. Use that mean yaw (≈0 when focus is centered) to rotate the camera toward
    // the bulk, keeping the fit centered on the union of all visors.
    fn camera_for_visor_arc(&self, instances: Vec<InstanceId>) -> Option<PixelCamera> {
        let transforms: Vec<Transform> = instances
            .iter()
            .map(|instance| {
                self.placement(&DesktopTarget::Instance(*instance))
                    .transform
            })
            .collect();
        let mean_yaw = transforms
            .iter()
            // Panels are pure Y-rotations (`from_rotation_y`), so yaw is recoverable from the
            // quaternion's y/w components without an euler-rotation dependency.
            .map(|t| 2.0 * t.rotate.y.atan2(t.rotate.w))
            .sum::<f64>()
            / transforms.len() as f64;

        let bounds = self.fold_instance_bounds(instances);
        self.camera_for_bounds(bounds, mean_yaw)
    }

    fn camera_for_bounds(&self, bounds: OverviewBounds, mean_yaw: f64) -> Option<PixelCamera> {
        // Point the camera at the 3D centroid of the visor corners (which carries the arc's z
        // offset, not the flat z=0 plane), rotated by the panels' mean yaw so it looks toward the
        // arc's bulk. The whole-set fit then measures projected extent around that center. An
        // empty point set yields None, letting the depth resolver fall back to an ancestor.
        let centroid = bounds.points.centroid()?;
        let look_at = Transform::new(centroid, Quaternion::from_rotation_y(mean_yaw), 1.0);
        let camera = look_at.to_camera();
        let distance = camera.fit_distance_for_points(&bounds.points, self.window_size)?;
        Some(camera.with_distance(distance))
    }

    // Frame only the instance panels, excluding the launcher's own background rect so the
    // overview doesn't span further left/right than the visible panels.
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

    // Band overview framing: axis-aligned flat panels only need the union of their rects — no
    // transformed corner points or mean yaw.
    fn fold_instance_rect(&self, instances: Vec<InstanceId>) -> Rect {
        instances
            .iter()
            .map(|instance| self.target_rect(&DesktopTarget::Instance(*instance)))
            .bounds()
            .expect("Internal error: a launcher with instances must yield rects")
    }

    /// The bounds of the matrix row, every assigned slot of that row included — a project-assigned
    /// slot widens it like a launcher does.
    fn matrix_row_rect_for_project(&self, project_id: ProjectId, row: u32) -> Option<Rect> {
        let mut rect: Option<Rect> = None;

        for (placement, content) in self.configuration.slots_ordered(project_id) {
            if placement.row != row {
                continue;
            }

            let slot_rect = self.target_rect(&content.target());

            rect = Some(match rect {
                Some(existing) => existing.joined(slot_rect),
                None => slot_rect,
            });
        }

        rect
    }

    fn camera_for_rect(&self, rect: Rect) -> Option<PixelCamera> {
        if rect.is_empty() {
            return None;
        }

        let center = rect.center();
        let center: Transform = (center.x, center.y, 0.0).into();
        let distance = Self::fit_letterbox_distance(rect.size(), self.window_size);
        Some(center.to_camera().with_distance(distance))
    }

    /// The letterboxing camera distance that fits `size` within the window.
    fn fit_letterbox_distance(size: Size, window_size: SizePx) -> f64 {
        let (surface_width, surface_height) = window_size.into();
        let scale_x = surface_width as f64 / size.width;
        let scale_y = surface_height as f64 / size.height;
        let fit_scale = scale_x.min(scale_y).max(f64::MIN_POSITIVE);
        PixelCamera::pixel_perfect_distance(PixelCamera::DEFAULT_FOVY) / fit_scale
    }

    fn project_rect(&self, project_id: ProjectId) -> Rect {
        let root = DesktopTarget::Project(project_id);
        let mut rect = Some(self.target_rect(&root));
        self.extend_rect_with_subtree(&root, &mut rect);
        rect.expect("Internal error: project bounds should always exist")
    }

    fn extend_rect_with_subtree(&self, root: &DesktopTarget, rect: &mut Option<Rect>) {
        for child in self.hierarchy.get_nested(root) {
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

    /// Build a camera that looks at the placement's full transform (translate + rotate),
    /// at the pixel-perfect distance.
    fn camera_from_placement(transform: Transform) -> PixelCamera {
        let look_at = Transform::new(transform.translate, transform.rotate, 1.0);
        look_at.to_camera().with_distance(
            PixelCamera::pixel_perfect_distance(PixelCamera::DEFAULT_FOVY) * transform.scale,
        )
    }

    fn placement(&self, target: &DesktopTarget) -> Placement<Transform, 2> {
        self.layout_state.absolute_placement(target, self.hierarchy)
    }
}

/// The extent of one or more placed targets: the axis-aligned `rect`, and the transformed corner
/// `points` the visor arc camera needs to fit rotated panels.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focused_scaled_content_keeps_its_local_pixel_scale() {
        let scale = 0.25;
        let camera = ZoomGeometry::camera_from_placement(Transform::from_scale(scale));

        assert_eq!(
            camera.distance,
            PixelCamera::pixel_perfect_distance(PixelCamera::DEFAULT_FOVY) * scale
        );
    }
}
