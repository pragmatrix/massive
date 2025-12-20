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

use crate::{Version, tools::Versioned};

#[derive(Debug)]
pub struct RenderGeometry {
    surface_size: SizePx,
    camera: PixelCamera,
    /// Dependencies tree head version.
    head_version: Version,
    /// Aggregated derived values cache.
    derived: RefCell<DerivedCache>,
}

const CAMERA_Z_RANGE: (f64, f64) = (0.1, 100.0);

impl RenderGeometry {
    pub fn new(surface_size: SizePx, camera: PixelCamera) -> Self {
        Self {
            surface_size,
            camera,
            head_version: 1,
            derived: RefCell::new(DerivedCache::default()),
        }
    }

    pub fn surface_size(&self) -> SizePx {
        self.surface_size
    }

    pub fn depth_range(&self) -> DepthRange {
        (0.0, 1.0).into()
    }

    /// Helper to transform screen coordinates to NDC coordinates.
    pub fn screen_to_ndc_matrix(&self) -> Matrix4 {
        let size = self.surface_size();
        let (w, h) = (size.width as f64, size.height as f64);
        Matrix4::from_cols_array(&[
            2.0 / w,
            0.0,
            0.0,
            -1.0,
            0.0,
            -2.0 / h,
            0.0,
            1.0,
            0.0,
            0.0,
            2.0,
            -1.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ])
    }

    pub fn camera(&self) -> &PixelCamera {
        &self.camera
    }

    pub fn set_surface_size(&mut self, surface_size: SizePx) {
        if self.surface_size != surface_size {
            self.surface_size = surface_size;
            self.head_version += 1;
        }
    }

    pub fn set_camera(&mut self, camera: PixelCamera) {
        if self.camera != camera {
            self.camera = camera;
            self.head_version += 1;
        }
    }

    /// Compute the final view projection. From pixel (3D) coordinate system to the final surface pixels.
    pub fn view_projection(&self) -> Matrix4 {
        let version = self.head_version;
        let mut derived = self.derived.borrow_mut();
        let vp = derived.model_to_surface(version, &self.camera, self.surface_size);
        *vp
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

    /// Un-projects a screen-space pixel position into model space at z==0 (the matrix describing a
    /// plane to hit).
    ///
    /// Returns the hit point in model-local coordinates or None if the ray is parallel or
    /// numerically unstable.
    pub fn unproject_to_model_z0(&self, pos_px: Point, model: &Matrix4) -> Option<Vector3> {
        let depth_range = self.depth_range();
        let mvp = self.view_projection() * *model;
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
    model_to_camera_to_ndc: Versioned<Matrix4>,
    camera_projection: Versioned<Matrix4>,
    view_projection: Versioned<Matrix4>,
}

impl DerivedCache {
    fn model_to_surface(
        &mut self,
        version: Version,
        camera: &PixelCamera,
        surface_size: SizePx,
    ) -> &Matrix4 {
        self.view_projection.resolve(version, || {
            let model_to_camera_to_ndc_matrix =
                self.model_to_camera_to_ndc.resolve(version, || {
                    RenderGeometry::model_to_ndc(surface_size) * camera.model_camera_matrix()
                });

            let camera_projection = self.camera_projection.resolve(version, || {
                let view_matrix = camera.ndc_camera_move();
                let perspective_matrix = camera.perspective_matrix(CAMERA_Z_RANGE, surface_size);
                perspective_matrix * view_matrix
            });

            *camera_projection * *model_to_camera_to_ndc_matrix
        })
    }
}
