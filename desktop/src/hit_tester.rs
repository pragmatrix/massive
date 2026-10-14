use massive_geometry::{Contains, Point, SizedTransform, Vector3};
use massive_renderer::RenderGeometry;

use crate::coordinate_mapper::{CoordinateMapper, PlacementSource};
use crate::projects::{RuntimeConfiguration, launcher_mode};
use crate::{DesktopTarget, HitTest, OrderedHierarchy};

/// Decides which target the pointer is over: which targets take part, overflowing children and
/// overlays, and the nearest hit. Mapping pointer positions into targets is the mapper's.
pub(crate) struct HitTester<'a> {
    /// Walked to find the targets to test. The mapper holds the same hierarchy for its own
    /// purpose, and getting it through the mapper would hide this dependency.
    hierarchy: &'a OrderedHierarchy<DesktopTarget>,
    configuration: &'a RuntimeConfiguration,
    mapper: CoordinateMapper<'a>,
}

#[derive(Debug)]
pub struct HitTestResult {
    pub target: DesktopTarget,
    pub local_pos: Vector3,
    pub surface_depth: f64,
}

impl HitTest<DesktopTarget> for HitTester<'_> {
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
                .map(|hit| (hit.target, hit.local_pos))
                .or_else(|| {
                    // Any position inside the window that misses all content maps to the
                    // desktop, so the pointer focus is never lost over empty margins. A
                    // cleared pointer focus (`None`) then means only that pointer feedback is
                    // suppressed (keyboard navigation active), which is what drives cursor
                    // visibility.
                    self.hit_test_target_plane(screen_pos, &DesktopTarget::Desktop)
                        .map(|local_pos| (DesktopTarget::Desktop, local_pos))
                }),
        }
    }
}

impl<'a> HitTester<'a> {
    pub fn new(
        hierarchy: &'a OrderedHierarchy<DesktopTarget>,
        placements: &'a dyn PlacementSource,
        configuration: &'a RuntimeConfiguration,
        geometry: &'a RenderGeometry,
    ) -> Self {
        Self {
            hierarchy,
            configuration,
            mapper: CoordinateMapper::new(hierarchy, placements, geometry),
        }
    }

    fn hit_test_target_plane(&self, screen_pos: Point, target: &DesktopTarget) -> Option<Vector3> {
        let hit_surface = self.mapper.surface(target)?;
        self.hit_test_surface(screen_pos, &hit_surface)
    }

    fn hit_test_hierarchy(&self, screen_pos: Point, root: &DesktopTarget) -> Option<HitTestResult> {
        let regular_hit = self.hit_test_hierarchy_with_depth(screen_pos, root, false);
        let overlay_hit = self.hit_test_overflow_overlays_with_depth(screen_pos);

        match (regular_hit, overlay_hit) {
            (Some(regular), Some(overlay)) => {
                Some(if overlay.surface_depth < regular.surface_depth {
                    overlay
                } else {
                    regular
                })
            }
            (Some(regular), None) => Some(regular),
            (None, Some(overlay)) => Some(overlay),
            (None, None) => None,
        }
    }

    fn hit_test_overflow_overlays_with_depth(&self, screen_pos: Point) -> Option<HitTestResult> {
        let mut topmost_hit: Option<HitTestResult> = None;

        for launcher in self.configuration.launchers() {
            if !launcher_mode::includes_overflow_children_in_hit_testing(launcher.mode) {
                continue;
            }

            let launcher_target = DesktopTarget::Launcher(launcher.id);
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
        let hit_surface = self.mapper.surface(root)?;
        let local_pos = self.hit_test_surface(screen_pos, &hit_surface)?;
        let hit_world_pos = hit_surface.transform.transform_point(local_pos);
        let hit_depth = self.mapper.depth(hit_world_pos);
        let is_inside_root = hit_surface
            .size
            .to_rect()
            .contains(Point::new(local_pos.x, local_pos.y));
        let include_children =
            is_inside_root || allow_overflow_children || root.children_may_overflow();

        if include_children {
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

    fn hit_test_surface(&self, screen_pos: Point, surface: &SizedTransform) -> Option<Vector3> {
        self.mapper.to_local(screen_pos, surface)
    }
}
