//! The full geometry of a renderer.
//!
//! This includes it's surface size up to the pixel view projection.
// Architecture: Might need move this up to the shell where the AsyncWindowRenderer uses it first.
// Architecture: This is slighly overengineered. Depedency tracking is probably not worth it.
use std::cell::RefCell;

use massive_geometry::{Camera, DepthRange, PerpectiveDivide, Plane, Point3, Ray, Vector4};
use massive_scene::Matrix;

use crate::{Version, tools::Versioned};

#[derive(Debug)]
pub struct RenderGeometry {
    surface_size: (u32, u32),
    camera: Camera,
    /// Dependencies tree head version.
    head_version: Version,
    /// Aggregated derived values cache.
    derived: RefCell<DerivedCache>,
}

const CAMERA_Z_RANGE: (f64, f64) = (0.1, 100.0);

impl RenderGeometry {
    pub fn new(surface_size: (u32, u32), camera: Camera) -> Self {
        Self {
            surface_size,
            camera,
            head_version: 1,
            derived: RefCell::new(DerivedCache::default()),
        }
    }

    pub fn surface_size(&self) -> (u32, u32) {
        self.surface_size
    }

    pub fn depth_range(&self) -> DepthRange {
        (0.0, 1.0).into()
    }

    /// Helper to transform screen coordinates to NDC coordinates.
    pub fn screen_to_ndc_matrix(&self) -> Matrix {
        let size = self.surface_size();
        let (w, h) = (size.0 as f64, size.1 as f64);
        Matrix::new(
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
        )
    }

    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    pub fn set_surface_size(&mut self, surface_size: (u32, u32)) {
        if self.surface_size != surface_size {
            self.surface_size = surface_size;
            self.head_version += 1;
        }
    }

    pub fn set_camera(&mut self, camera: Camera) {
        if self.camera != camera {
            self.camera = camera;
            self.head_version += 1;
        }
    }

    /// Compute the final view projection. From pixel (3D) coordinate system to the final surface pixels.
    pub fn view_projection(&self) -> Matrix {
        let version = self.head_version;
        let mut derived = self.derived.borrow_mut();
        let vp = derived.view_projection(version, &self.camera, self.surface_size);
        *vp
    }

    /// A Matrix that translates from pixels (0,0)-(width,height) to screen space, which is -1.0 to
    /// 1.0 in each axis. Also flips y.
    ///
    /// Precision: When the surface height changes, the whole perspective gets skewed
    fn pixel_matrix(surface_size: (u32, u32)) -> Matrix {
        let (_, surface_height) = surface_size;
        Matrix::from_nonuniform_scale(1.0, -1.0, 1.0)
            * Matrix::from_scale(1.0 / surface_height as f64 * 2.0)
    }

    /// Unprojects a screen-space pixel position into model space at z==0 (the matrix describing a
    /// plane to hit).
    ///
    /// Returns the hit point in model-local coordinates or None if the ray is parallel or
    /// numerically unstable.
    pub fn unproject_to_model_z0(&self, pos_px: (f64, f64), model: &Matrix) -> Option<Point3> {
        use cgmath::SquareMatrix;
        let depth_range = self.depth_range();
        let inverted_mvp = (self.view_projection() * model).invert()?;

        // Screen -> NDC (flip Y)
        let (ndc_x, ndc_y) = self.screen_to_ndc(pos_px);

        // Unproject near/far in panel space directly
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
    fn screen_to_ndc(&self, pos_px: (f64, f64)) -> (f64, f64) {
        let surface_size = self.surface_size();

        // Screen -> NDC (flip Y)
        let ndc_x = (pos_px.0 / surface_size.0 as f64) * 2.0 - 1.0;
        let ndc_y = 1.0 - (pos_px.1 / surface_size.1 as f64) * 2.0;
        (ndc_x, ndc_y)
    }
}

#[derive(Debug, Default)]
struct DerivedCache {
    pixel_matrix: Derived<Matrix>,
    camera_projection: Derived<Matrix>,
    view_projection: Derived<Matrix>,
}

impl DerivedCache {
    fn view_projection(
        &mut self,
        version: Version,
        camera: &Camera,
        surface_size: (u32, u32),
    ) -> &Matrix {
        self.view_projection.resolve(version, || {
            let pixel_matrix = self
                .pixel_matrix
                .resolve(version, || RenderGeometry::pixel_matrix(surface_size));

            let camera_projection = self.camera_projection.resolve(version, || {
                camera.view_projection_matrix(CAMERA_Z_RANGE, surface_size)
            });

            camera_projection * pixel_matrix
        })
    }
}

#[derive(Debug)]
pub struct Derived<T> {
    inner: Option<Versioned<T>>,
}

impl<T> Default for Derived<T> {
    fn default() -> Self {
        Self { inner: None }
    }
}

impl<T> Derived<T> {
    pub fn resolve(&mut self, head_version: Version, mut resolver: impl FnMut() -> T) -> &T {
        if self.inner.is_none() {
            self.inner = Some(Versioned::new(resolver(), head_version));
            self.inner.as_deref().unwrap()
        } else {
            self.inner.as_mut().unwrap().resolve(head_version, resolver)
        }
    }
}
