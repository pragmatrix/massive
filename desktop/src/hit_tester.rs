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

struct HitSurface {
    transform: Transform,
    size: Size,
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

    fn hit_test_hierarchy(
        &self,
        screen_pos: Point,
        root: &DesktopTarget,
    ) -> Option<(DesktopTarget, Vector3)> {
        self.hit_test_hierarchy_with_depth(screen_pos, root)
            .map(|(target, hit, _)| (target, hit))
    }

    fn hit_test_target_plane(&self, screen_pos: Point, target: &DesktopTarget) -> Option<Vector3> {
        let hit_surface = self.resolve_hit_surface(target)?;
        self.hit_test_surface(screen_pos, &hit_surface)
    }

    fn hit_test_hierarchy_with_depth(
        &self,
        screen_pos: Point,
        root: &DesktopTarget,
    ) -> Option<(DesktopTarget, Vector3, f64)> {
        let hit_surface = self.resolve_hit_surface(root)?;
        let local_pos = self.hit_test_surface(screen_pos, &hit_surface)?;
        let is_inside_root = hit_surface
            .size
            .to_rect()
            .contains(Point::new(local_pos.x, local_pos.y));
        let allow_overflow_children =
            self.is_in_visor_hierarchy(root) || self.has_visor_descendant(root);

        if is_inside_root || allow_overflow_children {
            let mut nearest_nested_hit: Option<(DesktopTarget, Vector3, f64)> = None;
            for nested in self.hierarchy.get_nested(root) {
                if let Some(target_hit) = self.hit_test_hierarchy_with_depth(screen_pos, nested)
                    && nearest_nested_hit
                        .as_ref()
                        .is_none_or(|(_, _, nearest_z)| target_hit.2 > *nearest_z)
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

            return Some((root.clone(), local_pos, hit_surface.transform.translate.z));
        }

        None
    }

    fn is_in_visor_hierarchy(&self, target: &DesktopTarget) -> bool {
        let mut current = Some(target);

        while let Some(node) = current {
            if let DesktopTarget::Launcher(launcher_id) = node {
                return self
                    .launchers
                    .get(launcher_id)
                    .is_some_and(|launcher| launcher.mode() == LauncherMode::Visor);
            }

            current = self.hierarchy.parent(node);
        }

        false
    }

    fn has_visor_descendant(&self, target: &DesktopTarget) -> bool {
        for nested in self.hierarchy.get_nested(target) {
            if let DesktopTarget::Launcher(launcher_id) = nested
                && self
                    .launchers
                    .get(launcher_id)
                    .is_some_and(|launcher| launcher.mode() == LauncherMode::Visor)
            {
                return true;
            }

            if self.has_visor_descendant(nested) {
                return true;
            }
        }

        false
    }

    fn resolve_hit_surface(&self, target: &DesktopTarget) -> Option<HitSurface> {
        let rect = self.layouter.rect(target).map(|rect| {
            let rect_px: RectPx = (*rect).into();
            Rect::from(rect_px)
        })?;

        let model = self.hit_test_transform(target, rect);

        Some(HitSurface {
            transform: model,
            size: rect.size(),
        })
    }

    fn hit_test_surface(&self, screen_pos: Point, hit_surface: &HitSurface) -> Option<Vector3> {
        self.geometry
            .unproject_to_model_z0(screen_pos, &hit_surface.transform.to_matrix4())
    }

    fn hit_test_transform(&self, target: &DesktopTarget, rect: Rect) -> Transform {
        if let Some(instance_id) = self.hit_target_instance_id(target) {
            return self
                .instances
                .get(&instance_id)
                .expect("Internal error: Missing instance presenter for hit test")
                .transform(rect);
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
            None => self.hit_test_hierarchy(screen_pos, &DesktopTarget::Desktop),
        }
    }
}
