use std::sync::Arc;

use derive_more::Debug;

use massive_geometry::{Rect, SizePx};

/// Draws the `source` pixel rect of an image into `rect`, so a caller can show any part of an
/// image, for example the cells of a terminal image that are still visible.
#[derive(Debug, Clone)]
pub struct Image {
    pub rect: Rect,
    /// In image pixels; fractional values sample between pixels.
    pub source: Rect,
    pub pixels: Arc<ImagePixels>,
}

impl Image {
    pub fn new(rect: Rect, source: Rect, pixels: Arc<ImagePixels>) -> Self {
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

/// Decoded pixels shared by every shape that shows a part of them. Renderers key their GPU copy by
/// the `Arc`'s address, so an image is uploaded once however many shapes draw from it.
#[derive(Debug)]
pub struct ImagePixels {
    size: SizePx,
    /// Straight-alpha RGBA8, row-major.
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

    pub fn size(&self) -> SizePx {
        self.size
    }

    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}
