use massive_applications::View;
use massive_applications::prelude::*;
use massive_geometry::{Size, SizePx, Transform};
use massive_scene::prelude::*;
use massive_scene::{Handle, Location, Ref};

/// The location at the center of an example's view.
///
/// An example centers its content around the origin, while a view's coordinates start at its
/// top-left corner. Content placed relative to this location stays centered when the view resizes,
/// in the shell's window as well as in the desktop.
#[derive(Debug)]
pub struct ViewCenter {
    transform: Handle<Transform>,
    location: Handle<Location>,
}

impl ViewCenter {
    pub fn new(view: &View, size: SizePx) -> Self {
        let transform = center_of(size).submit();
        let location = transform
            .to_location()
            .relative_to(view.location())
            .submit();
        Self {
            transform,
            location,
        }
    }

    pub fn location(&self) -> Ref<Location> {
        self.location.to_ref()
    }

    /// Follows the host's new extent of `view`.
    pub fn resize(&self, view: &View, size: SizePx) {
        view.set_extent(size);
        self.transform.update_if_changed(center_of(size));
    }
}

fn center_of(size: SizePx) -> Transform {
    let size: Size = size.into();
    Transform::from(size.center())
}
