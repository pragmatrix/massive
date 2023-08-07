use std::sync::Arc;

use derive_more::Constructor;
use granularity_geometry::Point3;

use crate::geometry::Camera;

pub enum Command {
    /// Draws an image with the given pipeline.
    ///
    /// The image is drawn with the given points as corners, counter clockwise, starting at left /
    /// top in relation to the image's texture data.
    DrawImage(Pipeline, [Point3; 4], Arc<ImageData>),
    DrawImageLazy(
        // Base Size point coordinates
        [Point3; 4],
        LazyImage,
        Box<dyn Fn() -> Arc<(Pipeline, [Point3; 4], ImageData)>>,
    ),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pipeline {
    Flat,
    Sdf,
}

#[derive(Debug, Constructor)]
pub struct PipelineTextureView {
    pipeline: Pipeline,
    texture_view: wgpu::TextureView,
    size: (u32, u32),
}

// TODO: Geometry candidate?
#[derive(Debug)]
pub struct ImageData {
    pub format: ImageFormat,
    pub size: (u32, u32),
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    A,
    Rgba,
}

impl ImageFormat {
    pub fn bytes_per_pixel(&self) -> usize {
        match self {
            ImageFormat::A => 1,
            ImageFormat::Rgba => 4,
        }
    }
}

#[derive(Debug)]
pub struct LazyImage {
    /// The base size the image data is generated.
    base_size: (u32, u32),
    /// Minimum Scale factor, 1: half as large, 2: quarter as large, etc., 0: stick to base.
    min_scale: u32,
    /// The maximum scale factor: 1: double, 2: quadruple, etc., 0: stick to base size.
    max_scale: u32,
    /// If rendering should be exact if the image is settled for a few frames and the image can be
    /// rendered pixel-exact.
    exact_if_settled: bool,
}
