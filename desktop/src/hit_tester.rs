use massive_applications::InstanceId;
use massive_geometry::{Contains, Point, Rect, RectPx, Size, Transform, Vector3};
use massive_layout::IncrementalLayouter;
use massive_renderer::RenderGeometry;

use crate::instance_presenter::InstancePresenter;
use crate::projects::{LaunchProfileId, LauncherPresenter};
use crate::{DesktopTarget, HitTester, Map, OrderedHierarchy};

pub(crate) struct AggregateHitTester<'a> {
    hierarchy: &'a OrderedHierarchy<DesktopTarget>,
    layouter: &'a IncrementalLayouter<DesktopTarget, Transform, 2>,
    launchers: &'a Map<LaunchProfileId, LauncherPresenter>,
    instances: &'a Map<InstanceId, InstancePresenter>,
    geometry: &'a RenderGeometry,
}

#[derive(Debug)]
struct HitSurface {
    transform: Transform,
    size: Size,
    surface_z: f64,
}

#[derive(Debug)]
pub struct HitTestResult {
    pub target: DesktopTarget,
    pub local_pos: Vector3,
    pub surface_z: f64,
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
        instances: &'a Map<InstanceId, InstancePresenter>,
        geometry: &'a RenderGeometry,
    ) -> Self {
        Self {
            hierarchy,
            layouter,
            launchers,
            instances,
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
            (Some(regular), Some(overlay)) => Some(if overlay.surface_z > regular.surface_z {
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
                        .is_none_or(|topmost| target_hit.surface_z > topmost.surface_z)
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
                        .is_none_or(|nearest| target_hit.surface_z > nearest.surface_z)
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
                surface_z: hit_surface.surface_z,
            });
        }

        None
    }

    fn resolve_hit_surface(&self, target: &DesktopTarget) -> Option<HitSurface> {
        let rect = self.layouter.rect(target).map(|rect| {
            let rect_px: RectPx = (*rect).into();
            Rect::from(rect_px)
        })?;

        let transform = self.hit_test_transform(target, rect);
        let surface_z = self.hit_surface_z(target);

        Some(HitSurface {
            transform,
            size: rect.size(),
            surface_z,
        })
    }

    fn hit_surface_z(&self, target: &DesktopTarget) -> f64 {
        // For instances and views, use the instance's layout transform z from the layouter.
        // We use the instance presenter's final_value() for hit depth stability during animations.
        if let Some(instance_id) = self.hit_target_instance_id(target) {
            return self
                .instances
                .get(&instance_id)
                .expect("Internal error: Missing instance presenter for hit test depth")
                .layout_transform_animation
                .final_value()
                .translate
                .z;
        }

        // For non-instance targets, use the layouter's stored transform.
        self.layouter
            .transform(target)
            .map_or(0.0, |t| t.translate.z)
    }

    fn hit_test_surface(&self, screen_pos: Point, hit_surface: &HitSurface) -> Option<Vector3> {
        self.geometry
            .unproject_to_model_z0(screen_pos, &hit_surface.transform.to_matrix4())
    }

    /// Returns a transform whose model space is the target's local coordinate system.
    /// Unprojecting through this transform yields target-local coordinates.
    fn hit_test_transform(&self, target: &DesktopTarget, rect: Rect) -> Transform {
        if let Some(instance_id) = self.hit_target_instance_id(target) {
            let instance_target = DesktopTarget::Instance(instance_id);
            let instance_transform = self
                .layouter
                .transform(&instance_target)
                .copied()
                .unwrap_or(Transform::IDENTITY);

            // For View targets, offset the instance transform by the view's position within the
            // instance so that unprojection returns view-local coordinates.
            let view_offset = match target {
                DesktopTarget::View(_) => {
                    let instance_rect = self.layouter.rect(&instance_target).map(|r| {
                        let r_px: RectPx = (*r).into();
                        Rect::from(r_px)
                    });
                    instance_rect.map_or(Vector3::ZERO, |ir| {
                        let offset = rect.origin() - ir.origin();
                        Vector3::new(offset.x, offset.y, 0.0)
                    })
                }
                _ => Vector3::ZERO,
            };

            let local_center = rect.size().to_rect().center();
            let mut layout_transform = instance_transform;
            layout_transform.translate =
                layout_transform.translate + layout_transform.rotate * view_offset;
            return InstancePresenter::transform_with_layout(layout_transform, local_center);
        }

        let origin = rect.origin();
        Transform::from_translation((origin.x, origin.y, 0.0))
    }

    fn hit_target_instance_id(&self, target: &DesktopTarget) -> Option<InstanceId> {
        Some(match target {
            DesktopTarget::Instance(instance_id) => *instance_id,
            DesktopTarget::View(_) => {
                let parent = self
                    .hierarchy
                    .parent(target)
                    .expect("Internal error: View without parent in hit test");

                match parent {
                    DesktopTarget::Instance(instance_id) => *instance_id,
                    _ => panic!("Internal error: View parent is not an instance in hit test"),
                }
            }
            DesktopTarget::Desktop | DesktopTarget::Group(_) | DesktopTarget::Launcher(_) => {
                return None;
            }
        })
    }
}
