use massive_applications::InstanceId;
use massive_geometry::{Contains, Point, Rect, RectPx, Size, Vector3};
use massive_layout::IncrementalLayouter;
use massive_renderer::RenderGeometry;
use massive_scene::Transform;

use crate::instance_presenter::InstancePresenter;
use crate::projects::{LaunchProfileId, LauncherMode, LauncherPresenter};
use crate::{DesktopTarget, HitTester, Map, OrderedHierarchy};

pub(crate) struct AggregateHitTester<'a> {
    hierarchy: &'a OrderedHierarchy<DesktopTarget>,
    layouter: &'a IncrementalLayouter<DesktopTarget, 2>,
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
        layouter: &'a IncrementalLayouter<DesktopTarget, 2>,
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
        let visor_overlay_hit = self.hit_test_visor_overlays_with_depth(screen_pos);

        match (regular_hit, visor_overlay_hit) {
            (Some(regular), Some(visor)) => Some(if visor.surface_z > regular.surface_z {
                visor
            } else {
                regular
            }),
            (Some(regular), None) => Some(regular),
            (None, Some(visor)) => Some(visor),
            (None, None) => None,
        }
    }

    fn hit_test_visor_overlays_with_depth(&self, screen_pos: Point) -> Option<HitTestResult> {
        let mut topmost_hit: Option<HitTestResult> = None;

        for (launcher_id, launcher) in self.launchers.iter() {
            if launcher.mode() != LauncherMode::Visor {
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

        let model = self.hit_test_transform(target, rect);
        let surface_z = self.hit_surface_z(target, &model);

        Some(HitSurface {
            transform: model,
            size: rect.size(),
            surface_z,
        })
    }

    fn hit_surface_z(&self, target: &DesktopTarget, model: &Transform) -> f64 {
        if let Some(instance_id) = self.hit_target_instance_id(target) {
            return self
                .instances
                .get(&instance_id)
                .expect("Internal error: Missing instance presenter for hit test depth")
                .layout_transform_animation
                // Keep hit depth stable across short structural animations.
                // We intentionally pick against the settled layout target for now.
                .final_value()
                .translate
                .z;
        }

        model.translate.z
    }

    fn hit_test_surface(&self, screen_pos: Point, hit_surface: &HitSurface) -> Option<Vector3> {
        self.geometry
            .unproject_to_model_z0(screen_pos, &hit_surface.transform.to_matrix4())
    }

    fn hit_test_transform(&self, target: &DesktopTarget, rect: Rect) -> Transform {
        if let Some(instance_id) = self.hit_target_instance_id(target) {
            // InstancePresenter::transform expects a local center (panel-local coordinates), not
            // the global layout center.
            let local_center = rect.size().to_rect().center();
            return self
                .instances
                .get(&instance_id)
                .expect("Internal error: Missing instance presenter for hit test")
                .transform(local_center);
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
