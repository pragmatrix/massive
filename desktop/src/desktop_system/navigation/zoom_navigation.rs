use massive_applications::InstanceId;
use massive_geometry::{BoundaryRect, Centroid, PixelCamera, Quaternion, Rect, RectPx, Vector3};
use massive_scene::prelude::*;

use crate::desktop_system::change::{Changes, DesktopChange, Zoom, set_focus};
use crate::desktop_system::{DesktopSystem, DesktopTarget, KeyboardFocusReason, ZoomLevel};
use crate::focus_path::PathResolver;
use crate::projects::{LauncherMode, MatrixPlacement, ProjectId, SlotContent};

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

impl DesktopSystem {
    /// Plans a zoom command (ADR 0018): steps the zoom level, and moves focus where the step
    /// crosses a project boundary.
    pub(crate) fn plan_zoom(&self, zoom: Zoom) -> Changes {
        let Some(focused) = self.event_router.keyboard_focus() else {
            return Changes::Empty;
        };
        let instance_target = is_instance_target(focused);
        let project = self.project_of(focused);
        // A target without a slot — the root project — has only one frame.
        let in_slot = self.slot_in_project(project, focused).is_some();

        match zoom {
            Zoom::Out if !in_slot => Changes::Empty,
            Zoom::Out => {
                let parent = self.aggregates.hierarchy.parent_project_of(project);
                match (self.zoom_level.zoom_out(instance_target), parent) {
                    // Leaving a nested project focuses it as a slot of its parent.
                    (Some(ZoomLevel::Project) | None, Some(_)) => {
                        self.focus_at_zoom_level(DesktopTarget::Project(project), ZoomLevel::Focus)
                    }
                    (Some(level), _) => self.set_zoom_level(level),
                    // The root project is the outermost frame.
                    (None, None) => Changes::Empty,
                }
            }
            Zoom::In => match self.zoom_level.zoom_in(instance_target).filter(|_| in_slot) {
                Some(level) => self.set_zoom_level(level),
                None => match project_of_project_target(focused) {
                    Some(entered) => self.plan_enter_project_slot(entered),
                    None => Changes::Empty,
                },
            },
            Zoom::Enter => {
                let target = match project_of_project_target(focused) {
                    Some(entered) => self.focus_target_for_project(entered),
                    None => focused.clone(),
                };
                self.focus_at_zoom_level(target, ZoomLevel::Focus)
            }
        }
    }

    /// Zooms into `project` by one level: focuses its focus slot, else its first slot, at `Row`.
    fn plan_enter_project_slot(&self, project: ProjectId) -> Changes {
        let Some(content) = self.last_focused_content(project).or_else(|| {
            self.aggregates
                .configuration
                .first_slot(project)
                .map(|(_, content)| content)
        }) else {
            return Changes::Empty;
        };
        // Enter at Row so one zoom step reveals the project's contents before narrowing to a
        // slot or instance. Restore the last focused slot to preserve the user's position.
        self.focus_at_zoom_level(self.slot_focus_target(content.target()), ZoomLevel::Row)
    }

    fn focus_at_zoom_level(&self, target: DesktopTarget, level: ZoomLevel) -> Changes {
        let mut changes = Changes::Empty;
        if self.event_router.keyboard_focus() != Some(&target) {
            changes += set_focus(Some(target), KeyboardFocusReason::Zoom);
        }
        changes += self.set_zoom_level(level);
        changes
    }

    fn set_zoom_level(&self, level: ZoomLevel) -> Changes {
        if level == self.zoom_level {
            return Changes::Empty;
        }
        DesktopChange::SetZoomLevel(level).into()
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

        match level.normalized(is_instance_target(target)) {
            ZoomLevel::Focus if is_instance_target(target) => self
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

    /// Whether the camera points at `target` itself at `level`.
    pub fn is_fully_zoomed_in(&self, target: &DesktopTarget, level: ZoomLevel) -> bool {
        level.normalized(is_instance_target(target)) == ZoomLevel::Focus
    }

    /// What the camera points at and the nesting depth of the zoom project (root = 0).
    ///
    /// `Focus` on a target other than an instance points at its slot and reads as `Slot`; a target
    /// without a slot points at its project.
    pub fn focused_zoom_level(&self) -> Option<(ZoomLevel, usize)> {
        self.event_router.keyboard_focus().map(|focused| {
            let project = self.project_of(focused);
            let level = match self.zoom_level.normalized(is_instance_target(focused)) {
                _ if self.slot_in_project(project, focused).is_none() => ZoomLevel::Project,
                ZoomLevel::Focus if !is_instance_target(focused) => ZoomLevel::Slot,
                level => level,
            };
            (
                level,
                self.aggregates.hierarchy.project_nesting_depth(project),
            )
        })
    }

    /// The project whose matrix holds `target`'s slot. A project target is a slot of its parent;
    /// the root project, which no matrix hosts, is its own zoom project.
    fn project_of(&self, target: &DesktopTarget) -> ProjectId {
        match project_of_project_target(target) {
            Some(project) => self
                .aggregates
                .hierarchy
                .parent_project_of(project)
                .unwrap_or(project),
            None => self.aggregates.hierarchy.project_of_target(target),
        }
    }

    fn slot_in_project(
        &self,
        project: ProjectId,
        target: &DesktopTarget,
    ) -> Option<(SlotContent, MatrixPlacement)> {
        let target_path = self.aggregates.hierarchy.resolve_path(Some(target));
        self.aggregates
            .configuration
            .slots_ordered(project)
            .find_map(|(placement, content)| {
                target_path
                    .contains(&content.target())
                    .then_some((content, placement))
            })
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
        let launcher_id = self.aggregates.hierarchy.launcher_of_target(target)?;

        let instances: Vec<_> = self
            .aggregates
            .hierarchy
            .launcher_instances(launcher_id)
            .collect();
        if instances.len() <= 1 {
            // A launcher with zero or one visor has no arc to union — frame the launcher itself.
            return self.camera_for_target(&DesktopTarget::Launcher(launcher_id));
        }

        // The band/visor camera split follows the configuration's mode; a launcher
        // outside the configuration falls through to the visor camera, the default mode.
        match self
            .aggregates
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
        let distance =
            camera.fit_distance_for_points(&bounds.points, self.window_state.inner_size)?;
        Some(camera.with_distance(distance))
    }

    fn camera_for_rect(&self, rect: Rect) -> Option<PixelCamera> {
        if rect.is_empty() {
            return None;
        }

        let center = rect.center();
        let center: Transform = (center.x, center.y, 0.0).into();
        let distance = Self::fit_letterbox_distance(rect.size(), self.window_state.inner_size);
        Some(center.to_camera().with_distance(distance))
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

    /// The bounds of the matrix row, every assigned slot of that row included — a project-assigned
    /// slot widens it like a launcher does.
    fn matrix_row_rect_for_project(&self, project_id: ProjectId, row: u32) -> Option<Rect> {
        let mut rect: Option<Rect> = None;

        for (placement, content) in self.aggregates.configuration.slots_ordered(project_id) {
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

    pub(super) fn project_rect(&self, project_id: ProjectId) -> Rect {
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

    // Band overview framing: axis-aligned flat panels only need the union of their rects — no
    // transformed corner points or mean yaw.
    fn fold_instance_rect(&self, instances: Vec<InstanceId>) -> Rect {
        instances
            .iter()
            .map(|instance| self.target_rect(&DesktopTarget::Instance(*instance)))
            .bounds()
            .expect("Internal error: a launcher with instances must yield rects")
    }
}

fn is_instance_target(target: &DesktopTarget) -> bool {
    matches!(
        target,
        DesktopTarget::Instance(_) | DesktopTarget::InstanceTitleBar(_) | DesktopTarget::View(_)
    )
}

/// The project a project-level target stands for; `Desktop` stands for the root project.
fn project_of_project_target(target: &DesktopTarget) -> Option<ProjectId> {
    match target {
        DesktopTarget::Project(project)
        | DesktopTarget::ProjectHeader(project)
        | DesktopTarget::ProjectMatrix(project) => Some(*project),
        DesktopTarget::Desktop => Some(ProjectId::ROOT),
        DesktopTarget::Launcher(_)
        | DesktopTarget::Instance(_)
        | DesktopTarget::InstanceTitleBar(_)
        | DesktopTarget::View(_) => None,
    }
}
