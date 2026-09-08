use crate::{Matrix4, Projection, SizePx, Transform, Vector3};

/// A pixel camera.
///
/// Detail: The camera is not expressed as its position, but at the point it is pointing to in model
/// coordinates.
///
/// Internally the (pixel) model is transformed so that pixel point the camera is looking at is at
/// 0,0, then everything is projected in the NDC (normalized device coordinates), and then the world
/// is moved back so that the surface pixels match the original pixel space.
#[derive(Debug, Clone, PartialEq, Copy)]
pub struct PixelCamera {
    /// The point the camera points at in model / pixel space.
    pub look_at: Transform,
    /// The distance from the camera's look-at point back along the view axis. `camera_distance()`
    /// is the pixel-perfect distance (where model pixels map 1:1 onto the surface); larger values
    /// dolly the camera back and shrink content. Zoom is a dolly in depth, not a model scale.
    pub distance: f64,
    pub fovy: f64,
}

impl Default for PixelCamera {
    fn default() -> Self {
        Self::look_at(
            Transform::IDENTITY,
            Self::camera_distance(Self::DEFAULT_FOVY),
            Self::DEFAULT_FOVY,
        )
    }
}

impl PixelCamera {
    pub const DEFAULT_FOVY: f64 = 45.0;

    /// The pixel-perfect distance for this camera's field of view.
    fn pixel_perfect_distance(&self) -> f64 {
        Self::camera_distance(self.fovy)
    }

    /// The pixel-perfect camera distance for a field of view: the distance at which model pixels
    /// map 1:1 onto the surface.
    pub fn camera_distance(fovy: f64) -> f64 {
        1.0 / (fovy / 2.0).to_radians().tan()
    }

    /// Create a new camera from a transform, a resolved distance, and field of view.
    ///
    /// `distance == camera_distance(fovy)` is pixel-perfect; larger values dolly back and zoom out.
    pub fn look_at(look_at: Transform, distance: f64, fovy: f64) -> Self {
        Self {
            look_at,
            distance,
            fovy,
        }
    }

    pub fn with_distance(mut self, distance: f64) -> Self {
        self.distance = distance;
        self
    }

    /// The matrix that moves the model so that the camera target is at 0,0. World-only: the
    /// pixel-perfect distance and any dolly live in [`ndc_camera_move`], kept separate from
    /// camera-space content (which is not dollied).
    pub fn model_camera_matrix(&self) -> Matrix4 {
        self.look_at.inverse().to_matrix4()
    }

    /// Move the model back along the camera axis so that its pointed-to position is visible at the
    /// camera's distance. World projection dollies by `distance` (which is `camera_distance` when
    /// pixel-perfect); camera-space content uses [`pixel_perfect_ndc_camera_move`] to stay fixed.
    pub fn ndc_camera_move(&self) -> Matrix4 {
        Matrix4::from_translation(-Vector3::new(0.0, 0.0, self.distance))
    }

    /// The [`ndc_camera_move`] at the fixed pixel-perfect distance, used for camera-space content
    /// that must remain on-screen regardless of world zoom.
    pub fn pixel_perfect_ndc_camera_move(&self) -> Matrix4 {
        Matrix4::from_translation(-Vector3::new(0.0, 0.0, self.pixel_perfect_distance()))
    }

    /// The matrix that projects NDC 3D coordinates to the final surface coordinates "2D".
    ///
    /// Architecture: If we internally use pixel coordinates, then go through NDC and here back in
    /// "2D". Is there a more direct way?
    pub fn perspective_matrix(
        &self,
        z_range: (f64, f64),
        surface_size: impl Into<SizePx>,
    ) -> Matrix4 {
        let (width, height) = surface_size.into().into();
        Projection::new(width as f64 / height as f64, z_range).perspective_matrix(self.fovy)
    }

    /// The camera distance that exactly frames `points` within `surface_size`, or `None` for an
    /// empty point set (the caller decides the fallback).
    ///
    /// Solves the on-screen constraint per point: a point at camera-space `(px, py, pz)` projects to
    /// `camera_distance * (px, py) / (d - pz_ndc)` at distance `d`, with the pixel depth converted
    /// into the dolly's NDC z units (`pz_ndc = pz / half_height`), so the binding distance is
    /// `d >= pz_ndc + camera_distance * |p| / half`. Points in front of the focal plane (`pz > 0`,
    /// closer to the camera) project larger and bind the fit; points behind it shrink and never
    /// bind. Exact for depth-spanning sets (the visor arc), not just flat content. No bisection
    /// solver.
    pub fn fit_distance_for_points(&self, points: &[Vector3], surface_size: SizePx) -> Option<f64> {
        if points.is_empty() {
            return None;
        }

        let (surface_width, surface_height) = surface_size.into();
        let half_width = surface_width as f64 * 0.5;
        let half_height = surface_height as f64 * 0.5;
        let to_camera = self.look_at.inverse();
        let camera_distance = self.pixel_perfect_distance();

        // The minimum dolly distance that keeps every point on-screen. A point at camera-space
        // (px, py, pz) projects to `camera_distance * (px, py) / (d - pz_ndc)` at distance `d`. The
        // pixel depth enters in the dolly's NDC z units: the NDC transform scales all axes by
        // 2/height, so `pz_ndc = pz / half_height` (adding raw pixel depth inflates the distance by
        // ~half_height and dollies out absurdly far). Solving
        // `|camera_distance * px / (d - pz_ndc)| <= half_width` for `d` gives
        // `d >= pz_ndc + camera_distance * |px| / half_width`. Points in front of the focal plane
        // (pz > 0, closer to the camera) project larger and bind the fit; points behind it shrink
        // and never bind. Exact for depth-spanning sets (the visor arc), not just flat content.
        let mut distance: f64 = 0.0;
        for point in points {
            let camera_point = to_camera.transform_point(*point);
            let pz_ndc: f64 = camera_point.z / half_height;
            let x: f64 = camera_point.x.abs() / half_width;
            let y: f64 = camera_point.y.abs() / half_height;
            distance = distance.max(pz_ndc + camera_distance * x.max(y));
        }
        Some(distance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_point_at_the_focal_plane_binds_at_pixel_perfect() {
        let camera = PixelCamera::default();
        let surface = SizePx::new(1920, 1080);
        let distance = camera
            .fit_distance_for_points(&[Vector3::new(960.0, 0.0, 0.0)], surface)
            .expect("non-empty points yield a distance");
        let pixel_perfect = PixelCamera::camera_distance(PixelCamera::DEFAULT_FOVY);
        assert!((distance - pixel_perfect).abs() < 1e-9);
    }

    #[test]
    fn points_in_front_of_the_focal_plane_bind_beyond_the_flat_fit() {
        let camera = PixelCamera::default();
        let surface = SizePx::new(1920, 1080);
        let flat = camera
            .fit_distance_for_points(&[Vector3::new(960.0, 0.0, 0.0)], surface)
            .expect("non-empty points yield a distance");
        let near = camera
            .fit_distance_for_points(&[Vector3::new(960.0, 0.0, 108.0)], surface)
            .expect("non-empty points yield a distance");
        assert!(near > flat);
    }

    #[test]
    fn points_behind_the_focal_plane_never_bind() {
        let camera = PixelCamera::default();
        let surface = SizePx::new(1920, 1080);
        let flat = camera
            .fit_distance_for_points(&[Vector3::new(960.0, 0.0, 0.0)], surface)
            .expect("non-empty points yield a distance");
        let behind = camera
            .fit_distance_for_points(&[Vector3::new(960.0, 0.0, -540.0)], surface)
            .expect("non-empty points yield a distance");
        assert!(behind < flat);
    }

    #[test]
    fn empty_points_yield_none() {
        let camera = PixelCamera::default();
        assert!(camera
            .fit_distance_for_points(&[], SizePx::new(1920, 1080))
            .is_none());
    }
}
