use massive_geometry::{
    Contains, PerspectiveDivide, Point, Rect, RectPx, Size, Transform, Vector3, Vector4,
};
use massive_layout::IncrementalLayouter;
use massive_renderer::RenderGeometry;

use crate::projects::{LaunchProfileId, LauncherPresenter};
use crate::{DesktopTarget, HitTester, Map, OrderedHierarchy};

pub(crate) struct AggregateHitTester<'a> {
    hierarchy: &'a OrderedHierarchy<DesktopTarget>,
    layouter: &'a IncrementalLayouter<DesktopTarget, Transform, 2>,
    launchers: &'a Map<LaunchProfileId, LauncherPresenter>,
    geometry: &'a RenderGeometry,
}

#[derive(Debug)]
struct HitSurface {
    transform: Transform,
    size: Size,
}

#[derive(Debug)]
pub struct HitTestResult {
    pub target: DesktopTarget,
    pub local_pos: Vector3,
    pub surface_depth: f64,
}

impl HitTester<DesktopTarget> for AggregateHitTester<'_> {
    fn hit_test(
        &self,
        screen_pos: Point,
        target: Option<&DesktopTarget>,
    ) -> Option<(DesktopTarget, Vector3)> {
        match target {
            Some(target) => self
                .hit_test_target_plane(screen_pos, target)
                .map(|hit| (target.clone(), hit)),
            None => self
                .hit_test_hierarchy(screen_pos, &DesktopTarget::Desktop)
                .map(|hit| (hit.target, hit.local_pos)),
        }
    }
}

impl<'a> AggregateHitTester<'a> {
    pub fn new(
        hierarchy: &'a OrderedHierarchy<DesktopTarget>,
        layouter: &'a IncrementalLayouter<DesktopTarget, Transform, 2>,
        launchers: &'a Map<LaunchProfileId, LauncherPresenter>,
        geometry: &'a RenderGeometry,
    ) -> Self {
        Self {
            hierarchy,
            layouter,
            launchers,
            geometry,
        }
    }

    fn hit_test_target_plane(&self, screen_pos: Point, target: &DesktopTarget) -> Option<Vector3> {
        let hit_surface = self.resolve_hit_surface(target)?;
        self.hit_test_surface(screen_pos, &hit_surface)
    }

    fn hit_test_hierarchy(&self, screen_pos: Point, root: &DesktopTarget) -> Option<HitTestResult> {
        let regular_hit = self.hit_test_hierarchy_with_depth(screen_pos, root, false);
        let overlay_hit = self.hit_test_overflow_overlays_with_depth(screen_pos);

        match (regular_hit, overlay_hit) {
            (Some(regular), Some(overlay)) => Some(if overlay.surface_depth < regular.surface_depth {
                overlay
            } else {
                regular
            }),
            (Some(regular), None) => Some(regular),
            (None, Some(overlay)) => Some(overlay),
            (None, None) => None,
        }
    }

    fn hit_test_overflow_overlays_with_depth(&self, screen_pos: Point) -> Option<HitTestResult> {
        let mut topmost_hit: Option<HitTestResult> = None;

        for (launcher_id, launcher) in self.launchers.iter() {
            if !launcher.includes_overflow_children_in_hit_testing() {
                continue;
            }

            let launcher_target = DesktopTarget::Launcher(*launcher_id);
            if !self.hierarchy.exists(&launcher_target) {
                continue;
            }

            for nested in self.hierarchy.get_nested(&launcher_target) {
                if let Some(target_hit) =
                    self.hit_test_hierarchy_with_depth(screen_pos, nested, true)
                    && topmost_hit
                        .as_ref()
                        .is_none_or(|topmost| target_hit.surface_depth < topmost.surface_depth)
                {
                    topmost_hit = Some(target_hit);
                }
            }
        }

        topmost_hit
    }

    fn hit_test_hierarchy_with_depth(
        &self,
        screen_pos: Point,
        root: &DesktopTarget,
        allow_overflow_children: bool,
    ) -> Option<HitTestResult> {
        let hit_surface = self.resolve_hit_surface(root)?;
        let local_pos = self.hit_test_surface(screen_pos, &hit_surface)?;
        let hit_world_pos = hit_surface.transform.transform_point(local_pos);
        let hit_depth = self.hit_depth(hit_world_pos);
        let is_inside_root = hit_surface
            .size
            .to_rect()
            .contains(Point::new(local_pos.x, local_pos.y));

        if is_inside_root || allow_overflow_children {
            let mut nearest_nested_hit: Option<HitTestResult> = None;
            for nested in self.hierarchy.get_nested(root) {
                if let Some(target_hit) =
                    self.hit_test_hierarchy_with_depth(screen_pos, nested, allow_overflow_children)
                    && nearest_nested_hit
                        .as_ref()
                        .is_none_or(|nearest| target_hit.surface_depth < nearest.surface_depth)
                {
                    nearest_nested_hit = Some(target_hit);
                }
            }
            if let Some(hit) = nearest_nested_hit {
                return Some(hit);
            }

            if !is_inside_root {
                return None;
            }

            return Some(HitTestResult {
                target: root.clone(),
                local_pos,
                surface_depth: hit_depth,
            });
        }

        None
    }

    fn resolve_hit_surface(&self, target: &DesktopTarget) -> Option<HitSurface> {
        let rect = self.layouter.rect(target).map(|rect| {
            let rect_px: RectPx = (*rect).into();
            Rect::from(rect_px)
        })?;
        let size = rect.size();

        let transform = self.hit_test_transform(target, size);
        Some(HitSurface { transform, size })
    }

    fn hit_test_surface(&self, screen_pos: Point, hit_surface: &HitSurface) -> Option<Vector3> {
        self.geometry
            .unproject_to_model_z0(screen_pos, &hit_surface.transform.to_matrix4())
    }

    /// Returns a transform whose model space is the target's local coordinate system.
    /// Unprojecting through this transform yields target-local coordinates.
    fn hit_test_transform(&self, target: &DesktopTarget, size: Size) -> Transform {
        let local_center = size.to_rect().center();

        // The Desktop is the layout root — its transform is T::default() (IDENTITY), not
        // center-based. Derive its origin from the rect offset directly.
        if let DesktopTarget::Desktop = target {
            let offset = self
                .layouter
                .rect(target)
                .map(|r| r.offset)
                .unwrap_or_default();
            return Transform::from_translation((offset[0] as f64, offset[1] as f64, 0.0));
        }

        // For View targets, resolve through the parent instance and apply the view's offset
        // within the instance so that unprojection returns view-local coordinates.
        if let DesktopTarget::View(_) = target {
            let instance_id = match self.hierarchy.parent(target) {
                Some(DesktopTarget::Instance(id)) => *id,
                Some(_) => panic!("Internal error: View parent is not an instance in hit test"),
                None => panic!("Internal error: View without parent in hit test"),
            };
            let instance_target = DesktopTarget::Instance(instance_id);
            let instance_transform = self
                .layouter
                .transform(&instance_target)
                .copied()
                .unwrap_or(Transform::IDENTITY);

            let instance_rect = self
                .layouter
                .rect(&instance_target)
                .copied()
                .expect("Internal error: Missing instance rect in hit test");
            let view_rect = self
                .layouter
                .rect(target)
                .copied()
                .expect("Internal error: Missing view rect in hit test");

            let instance_center = Vector3::new(
                instance_rect.offset[0] as f64 + instance_rect.size[0] as f64 / 2.0,
                instance_rect.offset[1] as f64 + instance_rect.size[1] as f64 / 2.0,
                0.0,
            );
            let view_center = Vector3::new(
                view_rect.offset[0] as f64 + view_rect.size[0] as f64 / 2.0,
                view_rect.offset[1] as f64 + view_rect.size[1] as f64 / 2.0,
                0.0,
            );
            let view_offset = view_center - instance_center;

            let mut layout_transform = instance_transform;
            layout_transform.translate =
                layout_transform.translate + layout_transform.rotate * view_offset;
            return Self::transform_with_layout(layout_transform, local_center);
        }

        let layout_transform = self
            .layouter
            .transform(target)
            .copied()
            .unwrap_or(Transform::IDENTITY);
        Self::transform_with_layout(layout_transform, local_center)
    }

    fn transform_with_layout(layout_transform: Transform, local_center: Point) -> Transform {
        let local_center = Vector3::new(local_center.x, local_center.y, 0.0);
        let origin_translation =
            layout_transform.translate - layout_transform.rotate * local_center;
        Transform::new(
            origin_translation,
            layout_transform.rotate,
            layout_transform.scale,
        )
    }

    fn hit_depth(&self, world_pos: Vector3) -> f64 {
        let vp = self.geometry.view_projection();
        let clip = vp * Vector4::new(world_pos.x, world_pos.y, world_pos.z, 1.0);
        clip.perspective_divide().map_or(f64::INFINITY, |ndc| ndc.z)
    }

}
