use std::fmt;
use std::sync::Arc;

use derive_more::Debug;

use massive_geometry::{Rect, SizePx};

use crate::Shape;

/// Draws the `source` pixel rect of an image into `rect`, so a caller can show any part of an
/// image, for example the cells of a terminal image that are still visible.
#[derive(Debug, Clone)]
pub struct Image {
    pub rect: Rect,
    /// In image pixels; fractional values sample between pixels.
    pub source: Rect,
    pub pixels: Arc<dyn ImageSource>,
}

impl Image {
    pub fn new(rect: Rect, source: Rect, pixels: Arc<dyn ImageSource>) -> Self {
        Self {
            rect,
            source,
            pixels,
        }
    }
}

impl PartialEq for Image {
    // Pixels compare by identity, comparing megabytes would make every scene diff expensive.
    fn eq(&self, other: &Self) -> bool {
        self.rect == other.rect
            && self.source == other.source
            && Arc::ptr_eq(&self.pixels, &other.pixels)
    }
}

/// The pixels an [`Image`] draws from: straight-alpha RGBA8, row-major. A trait, so callers can
/// hand over pixels they already hold, such as a terminal engine's, without copying them.
/// Renderers key their GPU copy by the `Arc`'s address, so a source is uploaded once however many
/// shapes draw from it. Its pixels must not change; new pixels need a new source.
pub trait ImageSource: fmt::Debug + Send + Sync {
    /// Calls `f` with the size and all pixels; a callback because sources may hold them behind a
    /// lock, and reading both under it keeps them consistent.
    fn read(&self, f: &mut dyn FnMut(SizePx, &[u8]));
}

/// Pixels owned outright, for callers that decode or generate them.
#[derive(Debug)]
pub struct ImagePixels {
    size: SizePx,
    #[debug(skip)]
    rgba: Vec<u8>,
}

impl ImagePixels {
    pub fn new(size: SizePx, rgba: Vec<u8>) -> Self {
        assert_eq!(
            rgba.len(),
            size.width as usize * size.height as usize * 4,
            "RGBA8 pixel count must match the image size"
        );
        Self { size, rgba }
    }
}

impl ImageSource for ImagePixels {
    fn read(&self, f: &mut dyn FnMut(SizePx, &[u8])) {
        f(self.size, &self.rgba)
    }
}

impl From<Image> for Shape {
    fn from(image: Image) -> Self {
        Shape::Image(Box::new(image))
    }
}
