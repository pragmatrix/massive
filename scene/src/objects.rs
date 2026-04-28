use std::sync::Arc;

use massive_geometry::{Bounds, Transform};
use massive_shapes::{GlyphRun, Shape};

use crate::{Change, Handle, Id, Object, SceneChange};

/// A visual represents a set of shapes that have a common position / location in the space.
///
/// Architecture: This has now the same size as [`VisualRenderObj`]. Why not just clone this one for
/// the renderer then .. or even just the [`Handle<Visual>`]?
///
/// Detail: `Clone` was added for `Handle::update_with_if_changed()`.
#[derive(Debug, Clone, PartialEq)]
pub struct Visual {
    pub location: Handle<Location>,
    /// Optional decal ordering value for this visual.
    ///
    /// If set, the renderer treats this visual as a decal and renders it in decal order using the
    /// decal pipeline configuration.
    ///
    /// Decals are drawn with depth testing but without z-buffer writes.
    ///
    /// Decal layers render after non-decal visuals in ascending order, so `decal_order = 0` is
    /// the first decal layer.
    pub decal_order: Option<usize>,

    /// An optional clip bounds in model space 2D only.
    pub clip_bounds: Option<Bounds>,

    /// DR: Clients should be able to use [`Visual`] directly as a an abstract thing. Like for
    /// example a line which contains multiple Shapes (runs, quads, etc.). Therefore `Vec<Shape>`
    /// and not just `Shape`.
    ///
    /// DI: Another idea is to add `Shape::Combined(Vec<Shape>)`, but this makes extraction per
    /// renderer a bit more complex. This would also point to sharing Shapes as handles ... which
    /// could go in direction of layout?
    ///
    /// Arc is used here to make sharing shapes with the renderer really cheap.
    pub shapes: Arc<[Shape]>,
}

impl Visual {
    pub fn new(location: Handle<Location>, shapes: impl Into<Arc<[Shape]>>) -> Self {
        Self {
            location,
            decal_order: None,
            clip_bounds: None,
            shapes: shapes.into(),
        }
    }

    pub fn with_decal_order(self, decal_order: usize) -> Self {
        Self {
            decal_order: Some(decal_order),
            ..self
        }
    }

    pub fn with_clip_bounds(self, bounds: impl Into<Bounds>) -> Self {
        Self {
            clip_bounds: Some(bounds.into()),
            ..self
        }
    }
}

#[derive(Debug, Clone)]
pub struct VisualRenderObj {
    pub location: Id,
    pub decal_order: Option<usize>,
    pub clip_bounds: Option<Bounds>,
    pub shapes: Arc<[Shape]>,
}

impl VisualRenderObj {
    pub fn runs(&self) -> impl Iterator<Item = &GlyphRun> {
        self.shapes.iter().filter_map(|s| {
            if let Shape::GlyphRun(run) = s {
                Some(run)
            } else {
                None
            }
        })
    }
}

impl Object for Visual {
    // And upload the render shape.
    type Change = VisualRenderObj;

    fn to_change(&self) -> Self::Change {
        VisualRenderObj {
            location: self.location.id(),
            decal_order: self.decal_order,
            clip_bounds: self.clip_bounds,
            shapes: self.shapes.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Location {
    pub parent: Option<Handle<Location>>,
    pub transform: Handle<Transform>,
    pub alpha: f32,
}

impl From<Handle<Transform>> for Location {
    fn from(transform: Handle<Transform>) -> Self {
        Self {
            parent: None,
            transform,
            alpha: 1.0,
        }
    }
}

impl Location {
    pub fn new(parent: Option<Handle<Location>>, transform: impl Into<Handle<Transform>>) -> Self {
        Self {
            parent,
            transform: transform.into(),
            alpha: 1.0,
        }
    }

    pub fn relative_to(mut self, parent: impl Into<Handle<Location>>) -> Self {
        self.parent = Some(parent.into());
        self
    }

    pub fn with_alpha(mut self, alpha: f32) -> Self {
        self.alpha = normalize_alpha(alpha);
        self
    }
}

// This allows `Into<Handle<Location>>` to take either a reference or an owned handle.
impl<T> From<&Handle<T>> for Handle<T>
where
    T: Object,
    SceneChange: From<Change<T::Change>>,
{
    fn from(value: &Handle<T>) -> Self {
        Handle::clone(value)
    }
}

impl Object for Location {
    type Change = LocationRenderObj;

    fn to_change(&self) -> Self::Change {
        let parent = self.parent.as_ref().map(|p| p.id());
        let transform = self.transform.id();
        LocationRenderObj {
            parent,
            transform,
            alpha: normalize_alpha(self.alpha),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LocationRenderObj {
    pub parent: Option<Id>,
    pub transform: Id,
    pub alpha: f32,
}

fn normalize_alpha(alpha: f32) -> f32 {
    if alpha.is_finite() {
        alpha.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

impl Object for Transform {
    type Change = Self;

    fn to_change(&self) -> Self::Change {
        *self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scene;

    #[test]
    fn location_new_defaults_to_opaque_alpha() {
        let scene = Scene::new();
        let transform = Transform::IDENTITY.enter(&scene);
        let location = Location::new(None, transform);

        assert_eq!(location.alpha, 1.0);
        assert_eq!(location.to_change().alpha, 1.0);
    }

    #[test]
    fn location_alpha_is_normalized_when_set_and_uploaded() {
        let scene = Scene::new();
        let transform = Transform::IDENTITY.enter(&scene);

        assert_eq!(
            Location::new(None, transform.clone())
                .with_alpha(2.0)
                .to_change()
                .alpha,
            1.0
        );
        assert_eq!(
            Location::new(None, transform.clone())
                .with_alpha(-1.0)
                .to_change()
                .alpha,
            0.0
        );
        assert_eq!(
            Location::new(None, transform)
                .with_alpha(f32::NAN)
                .to_change()
                .alpha,
            1.0
        );
    }
}
