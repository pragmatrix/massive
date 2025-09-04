//! The full geometry of a renderer.
//!
//! This includes it's surface size up to the pixel view projection.
// Architecture: Might need move this up to the shell where the AsyncWindowRenderer uses it first.
// Architecture: This is slighly overengineered. Depedency tracking is probably not worth it.
use std::ops::Deref;

use massive_geometry::Camera;
use massive_scene::Matrix;

use crate::{Version, tools::Versioned};

#[derive(Debug)]
pub struct RenderGeometry {
    surface_size: (u32, u32),
    camera: Camera,
    /// Dependencies tree head version.
    head_version: Version,

    pixel_matrix: Derived<Matrix>,
    camera_projection: Derived<Matrix>,
    view_projection: Derived<Matrix>,
}

const Z_RANGE: (f64, f64) = (0.1, 100.0);

impl RenderGeometry {
    pub fn new(surface_size: (u32, u32), camera: Camera) -> Self {
        Self {
            surface_size,
            camera,
            head_version: 1,
            pixel_matrix: Default::default(),
            camera_projection: Default::default(),
            view_projection: Default::default(),
        }
    }

    pub fn surface_size(&self) -> (u32, u32) {
        self.surface_size
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
    pub fn view_projection(&mut self) -> Matrix {
        let version = self.head_version;
        *self.view_projection.resolve(version, || {
            let pixel_matrix = self
                .pixel_matrix
                .resolve(version, || Self::pixel_matrix(self.surface_size));

            let camera_matrix = self.camera_projection.resolve(version, || {
                self.camera
                    .view_projection_matrix(Z_RANGE, self.surface_size)
            });

            camera_matrix * *pixel_matrix
        })
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
}

#[derive(Debug)]
pub struct Derived<T> {
    inner: Option<Versioned<T>>,
}

impl<T> Default for Derived<T> {
    fn default() -> Self {
        Self {
            inner: Default::default(),
        }
    }
}

impl<T> Derived<T> {
    pub fn resolve(&mut self, head_version: Version, mut resolver: impl FnMut() -> T) -> &T {
        if self.inner.is_none() {
            self.inner = Some(Versioned::new(resolver(), head_version));
            self.inner.as_ref().unwrap().deref()
        } else {
            self.inner.as_mut().unwrap().resolve(head_version, resolver)
        }
    }
}
