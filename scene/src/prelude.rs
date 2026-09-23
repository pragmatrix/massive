//! Convenience re-exports of the ergonomic scene-content construction surface.
//!
//! Import with `use massive_scene::prelude::*;` to bring the chainable traits, the
//! [`identity_location`] free function, and the core content types into scope.

pub use crate::ergonomics::{
    At, IntoVisual, ToCamera, ToLocation, ToTransform, UnenteredLocation, VisualWithoutLocation,
    identity_location,
};
pub use crate::{Handle, Location, Object, Transform, Visual};
