use std::fmt;
use std::sync::Arc;

use derive_more::Debug;

use massive_geometry::{Rect, SizePx, TextureBox};

/// Draws the `region` of `source` into `rect`, so a caller can show any part of an
/// image, for example the cells of a terminal image that are still visible.
#[derive(Debug, Clone)]
pub struct Image {
    pub rect: Rect,
    pub region: TextureBox,
    pub source: Arc<dyn ImageSource>,
}

impl Image {
    pub fn new(rect: Rect, region: TextureBox, source: Arc<dyn ImageSource>) -> Self {
        Self {
            rect,
            region,
            source,
        }
    }
}

impl PartialEq for Image {
    // Sources compare by identity, comparing megabytes would make every scene diff expensive.
    fn eq(&self, other: &Self) -> bool {
        self.rect == other.rect
            && self.region == other.region
            && Arc::ptr_eq(&self.source, &other.source)
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
