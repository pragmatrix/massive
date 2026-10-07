//! The full geometry of a renderer.
//!
//! This includes it's surface size up to the pixel view projection.
// Architecture: Might need move this up to the shell where the AsyncWindowRenderer uses it first.
// Architecture: This is slightly over-engineered. Dependency tracking is probably not worth it.
use std::cell::RefCell;

use massive_geometry::{
    DepthRange, Matrix4, PerspectiveDivide, PixelCamera, Plane, Point, Ray, SizePx, Vector3,
    Vector4,
};
use massive_scene::LocationSpace;

use crate::{Version, tools::Versioned};

/// The per-space view projections used to render a frame.
///
/// Architecture: The projections are mathematically independent, but resolved as one versioned
/// value: rendering selects per visual every frame, so both are needed together in the only hot
/// path, and one version guarantees a frame's projections share the same camera and surface size.
/// Per-space lazy caching would add invalidation machinery to save ~3 matrix ops per frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewProjections {
    /// Project model (world) space to surface pixels, through the world camera.
    pub world: Matrix4,
    /// Project camera-space model coordinates to surface pixels, ignoring the world camera's
    /// position and target-size mode, but keeping the same perspective (fovy).
    pub camera: Matrix4,
}

#[derive(Debug)]
pub struct RenderGeometry {
    surface_size: SizePx,
    camera: PixelCamera,
    /// Dependencies tree head version.
    version: Version,
    /// Derived values.
    derived: RefCell<DerivedCache>,
}

const CAMERA_CLIP_RANGE: (f64, f64) = (0.1, 100.0);

impl RenderGeometry {
    pub fn new(surface_size: SizePx, camera: PixelCamera) -> Self {
        Self {
            surface_size,
            camera,
            version: 1,
            derived: RefCell::new(DerivedCache::default()),
        }
    }

    pub fn surface_size(&self) -> SizePx {
        self.surface_size
    }

    pub fn ndc_depth_range(&self) -> DepthRange {
        (0.0, 1.0).into()
    }

    pub fn camera(&self) -> &PixelCamera {
        &self.camera
    }

    pub fn set_surface_size(&mut self, surface_size: SizePx) {
        if self.surface_size != surface_size {
            self.surface_size = surface_size;
            self.version += 1;
        }
    }

    pub fn set_camera(&mut self, camera: PixelCamera) {
        if self.camera != camera {
            self.camera = camera;
            self.version += 1;
        }
    }

    /// Compute the final view projection. From pixel (3D) coordinate system to the final surface pixels.
    pub fn view_projection(&self) -> Matrix4 {
        self.view_projections().world
    }

    /// Compute the view projections for all coordinate spaces.
    pub fn view_projections(&self) -> ViewProjections {
        let version = self.version;
        let mut derived = self.derived.borrow_mut();
        *derived.view_projections.resolve(version, || {
            let world = Self::model_to_surface(&self.camera, self.surface_size);
            // Camera space shares the perspective but not the world camera's look-at and
            // target-size mode: camera-space content is positioned relative to the camera.
            let camera = Self::camera_space_projection(&self.camera, self.surface_size);
            ViewProjections { world, camera }
        })
    }

    /// A Matrix that translates from pixels (0,0)-(width,height) to screen space, which is -1.0 to
    /// 1.0 in each axis. Also flips y.
    ///
    /// Precision: When the surface height changes, the whole perspective gets skewed
    fn model_to_ndc(surface_size: SizePx) -> Matrix4 {
        let (_, surface_height) = surface_size.into();
        let scale = 2.0 / surface_height as f64;
        Matrix4::from_scale(Vector3::new(scale, -scale, scale))
    }

    fn model_to_surface(camera: &PixelCamera, surface_size: SizePx) -> Matrix4 {
        let model_to_camera_to_ndc_matrix =
            RenderGeometry::model_to_ndc(surface_size) * camera.model_camera_matrix();

        let view_matrix = camera.ndc_camera_move();
        let perspective_matrix = camera.perspective_matrix(world_clip_range(camera), surface_size);

        perspective_matrix * view_matrix * model_to_camera_to_ndc_matrix
    }

    /// The projection for camera-space coordinates: same perspective and pixel-to-NDC mapping as
    /// world space, but without the world camera's look-at, and at the fixed pixel-perfect distance
    /// so camera-space content (e.g. overlays) stays on-screen regardless of world zoom.
    fn camera_space_projection(camera: &PixelCamera, surface_size: SizePx) -> Matrix4 {
        let view_matrix = camera.pixel_perfect_ndc_camera_move();
        let perspective_matrix = camera.perspective_matrix(CAMERA_CLIP_RANGE, surface_size);
        let model_to_ndc = RenderGeometry::model_to_ndc(surface_size);

        perspective_matrix * view_matrix * model_to_ndc
    }

    /// Un-projects a screen-space pixel position into model space at z==0 (the matrix describing a
    /// plane to hit).
    ///
    /// Returns the hit point in model-local coordinates or None if the ray is parallel or
    /// numerically unstable.
    pub fn unproject_to_model_z0(
        &self,
        pos_px: Point,
        model: &Matrix4,
        space: LocationSpace,
    ) -> Option<Vector3> {
        let depth_range = self.ndc_depth_range();
        let projections = self.view_projections();
        let projection = match space {
            LocationSpace::World => projections.world,
            LocationSpace::Camera => projections.camera,
        };
        let mvp = projection * *model;
        // Note: The determinant can be very small (e.g., 1e-10) due to the coordinate system
        // scaling, but the matrix is still invertible. We rely on downstream checks
        // (perspective_divide, Ray::from_points, intersect_plane) to handle degenerate cases.
        let inverted_mvp = mvp.inverse();

        // Screen -> NDC (flip Y)
        let (ndc_x, ndc_y) = self.screen_to_ndc(pos_px).into();

        // Unproject near/far in plane space directly
        let clip_near = Vector4::new(ndc_x, ndc_y, depth_range.near, 1.0);
        let clip_far = Vector4::new(ndc_x, ndc_y, depth_range.far, 1.0);
        let near_h = inverted_mvp * clip_near;
        let far_h = inverted_mvp * clip_far;
        let near_p = near_h.perspective_divide()?;
        let far_p = far_h.perspective_divide()?;

        let ray = Ray::from_points(near_p, far_p)?;
        let plane = Plane::new((0.0, 0.0, 0.0), (0.0, 0.0, 1.0));
        ray.intersect_plane(&plane)
    }

    /// Map screen pixel coordinates to normalized WGPU device coordinates.
    fn screen_to_ndc(&self, pos_px: Point) -> Point {
        let surface_size = self.surface_size();

        // Screen -> NDC (flip Y)
        let ndc_x = (pos_px.x / surface_size.width as f64) * 2.0 - 1.0;
        let ndc_y = 1.0 - (pos_px.y / surface_size.height as f64) * 2.0;
        (ndc_x, ndc_y).into()
    }
}

#[derive(Debug, Default)]
struct DerivedCache {
    view_projections: Versioned<ViewProjections>,
}

impl Default for Versioned<ViewProjections> {
    fn default() -> Self {
        Self::new(
            ViewProjections {
                world: Matrix4::IDENTITY,
                camera: Matrix4::IDENTITY,
            },
            0,
        )
    }
}

/// The clip range for the world projection, scaled with the camera distance.
///
/// Deeply nested content is presented at a tiny scale, so the camera that frames it sits very close
/// to the focal plane. A fixed range would clip the scene there, and would lose depth precision
/// further out. Scaling both planes by the distance relative to the pixel-perfect distance keeps the
/// range identical at pixel-perfect zoom (where the decal depth bias is tuned) and the near/far
/// ratio constant everywhere else.
fn world_clip_range(camera: &PixelCamera) -> (f64, f64) {
    let (near, far) = CAMERA_CLIP_RANGE;
    let scale = camera.distance / PixelCamera::pixel_perfect_distance(camera.fovy);
    (near * scale, far * scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use massive_geometry::{Transform, Vector4};

    /// Whether the model origin (a point on the focal plane) survives depth clipping.
    fn origin_is_inside_depth_range(distance: f64) -> bool {
        let camera = PixelCamera::look_at(Transform::IDENTITY, distance, PixelCamera::DEFAULT_FOVY);
        let geometry = RenderGeometry::new(SizePx::new(1000, 700), camera);
        let clip = geometry.view_projection() * Vector4::new(0.0, 0.0, 0.0, 1.0);
        (0.0..=clip.w).contains(&clip.z)
    }

    /// A deeply nested project is presented at a tiny scale, so the camera that frames it sits
    /// very close to the focal plane. The focal plane must stay inside the clip range.
    #[test]
    fn focal_plane_is_not_clipped_at_small_camera_distances() {
        for distance in [500.0, 2.4, 0.5, 0.1, 0.05, 0.01, 0.001] {
            assert!(
                origin_is_inside_depth_range(distance),
                "focal plane clipped at camera distance {distance}"
            );
        }
    }

    #[test]
    fn clip_range_is_unchanged_at_the_pixel_perfect_distance() {
        let fovy = PixelCamera::DEFAULT_FOVY;
        let camera = PixelCamera::look_at(
            Transform::IDENTITY,
            PixelCamera::pixel_perfect_distance(fovy),
            fovy,
        );
        let (near, far) = world_clip_range(&camera);
        assert!((near - CAMERA_CLIP_RANGE.0).abs() < 1e-12);
        assert!((far - CAMERA_CLIP_RANGE.1).abs() < 1e-9);
    }

    #[test]
    fn clip_range_ratio_is_constant_across_distances() {
        let ratio = |distance: f64| {
            let camera =
                PixelCamera::look_at(Transform::IDENTITY, distance, PixelCamera::DEFAULT_FOVY);
            let (near, far) = world_clip_range(&camera);
            far / near
        };
        let reference = ratio(2.4);
        for distance in [0.001, 0.05, 10.0, 500.0] {
            assert!((ratio(distance) / reference - 1.0).abs() < 1e-9);
        }
    }
}
