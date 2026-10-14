//! Maps between surface pixels and a target's local coordinates, through the surface the target
//! is laid out on. Hit testing maps pointer positions to targets; the window maps a view's text
//! input area back to pixels for the IME. Both go through here, so the two directions agree.

use massive_geometry::{
    PerspectiveDivide, Point, Rect, RectPx, SizedTransform, Transform, Vector3, Vector4,
};
use massive_layout::{LayoutTopology, Placement};
use massive_renderer::RenderGeometry;
use massive_scene::LocationSpace;

use crate::{DesktopTarget, OrderedHierarchy};

pub(crate) trait PlacementSource {
    fn placement(
        &self,
        target: &DesktopTarget,
        hierarchy: &OrderedHierarchy<DesktopTarget>,
    ) -> Placement<Transform, 2>;
}

/// Built per use from the current layout and render geometry, by the hit tester and for the
/// window's IME area.
pub(crate) struct CoordinateMapper<'a> {
    /// Combines the relative placements into absolute ones and finds the layout root.
    hierarchy: &'a OrderedHierarchy<DesktopTarget>,
    placements: &'a dyn PlacementSource,
    geometry: &'a RenderGeometry,
}

impl<'a> CoordinateMapper<'a> {
    pub fn new(
        hierarchy: &'a OrderedHierarchy<DesktopTarget>,
        placements: &'a dyn PlacementSource,
        geometry: &'a RenderGeometry,
    ) -> Self {
        Self {
            hierarchy,
            placements,
            geometry,
        }
    }

    /// The target's surface: its size and a transform whose model space is the target's local
    /// coordinate system. `None` if the target is not visible.
    pub fn surface(&self, target: &DesktopTarget) -> Option<SizedTransform> {
        let placement = self.placements.placement(target, self.hierarchy);
        if !placement.visible {
            return None;
        }
        let rect_px: RectPx = placement.rect.into();
        let size = Rect::from(rect_px).size();

        let transform = self.local_transform(target, placement);
        Some(SizedTransform::new(size, transform))
    }

    /// The target-local point on the surface plane under a surface pixel.
    pub fn to_local(&self, surface_pos: Point, surface: &SizedTransform) -> Option<Vector3> {
        // Detail: All targets currently live in world space. Derive the space from the resolved
        // location when camera-space content becomes interactive.
        self.geometry.unproject_to_model_z0(
            surface_pos,
            &surface.transform.to_matrix4(),
            LocationSpace::World,
        )
    }

    /// The surface pixel a target-local point is presented at.
    pub fn to_surface(&self, local_pos: Point, surface: &SizedTransform) -> Point {
        let projected = self.geometry.project_to_surface(
            Vector3::new(local_pos.x, local_pos.y, 0.0),
            &surface.transform.to_matrix4(),
            LocationSpace::World,
        );
        projected.expect("A visible target's surface lies in front of the camera")
    }

    /// The depth a world point is presented at; smaller is nearer to the camera.
    pub fn depth(&self, world_pos: Vector3) -> f64 {
        let vp = self.geometry.view_projection();
        let clip = vp * Vector4::new(world_pos.x, world_pos.y, world_pos.z, 1.0);
        clip.perspective_divide().map_or(f64::INFINITY, |ndc| ndc.z)
    }

    /// Returns a transform whose model space is the target's local coordinate system.
    fn local_transform(
        &self,
        target: &DesktopTarget,
        placement: Placement<Transform, 2>,
    ) -> Transform {
        let rect_px: RectPx = placement.rect.into();
        let local_center = Rect::from(rect_px).size().to_rect().center();

        // The layout root — the parentless project — uses an origin-based transform
        // (IDENTITY in the common case). Derive its origin from the rectangle offset
        // directly.
        if self.hierarchy.parent_of(target).is_none() {
            let offset = placement.rect.offset;
            return Transform::from_xy(offset[0] as f64, offset[1] as f64);
        }

        placement.transform.to_origin_space(local_center)
    }
}
