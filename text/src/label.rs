use std::ops::DerefMut;

use anyhow::Result;
use cgmath::{Point2, Transform};
use cosmic_text as text;
use granularity::{map_ref, Value};
use granularity_geometry::{Bounds, Matrix4, Point3, Size3};
use granularity_shell::Shell;
use nearly::nearly_eq;
use text::SwashImage;
use wgpu::util::DeviceExt;

use crate::{
    distance_field_gen::{generate_distance_field_from_image, DISTANCE_FIELD_PAD},
    render_graph::{Pipeline, PipelineTextureView},
    Extent, TextureVertex,
};

pub struct Label {
    placed_glyphs: Value<(LabelMetrics, Vec<PositionedGlyph>)>,
    pub metrics: Value<LabelMetrics>,
}

pub struct RenderLabel {
    /// TODO: Separate?
    pub placements_and_texture_views: Value<Vec<Option<(text::Placement, PipelineTextureView)>>>,
    pub vertex_buffers: Value<Vec<Option<wgpu::Buffer>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LabelMetrics {
    pub max_ascent: u32,
    pub max_descent: u32,
    pub width: u32,
}

impl LabelMetrics {
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.max_ascent + self.max_descent)
    }
}

impl Extent for LabelMetrics {
    fn size(&self) -> Size3 {
        let (width, height) = self.size();
        Size3::new((width as f64, height as f64, 0.0).into())
    }

    fn baseline(&self) -> Option<f64> {
        Some(self.max_ascent as f64)
    }
}

pub fn new_label(shell: &Shell, font_size: Value<f32>, text: Value<String>) -> Label {
    let font_system = &shell.font_system;

    let metrics_and_placed_glyphs = map_ref!(|font_system, text, font_size| {
        let mut font_system = font_system.borrow_mut();
        let font_system = font_system.deref_mut();
        // TODO: Cosmic text recommends to use a single buffer for a widget, but we are creating a
        // new one every time the text change. Not sure if that makes a big difference, because it
        // seems that all the shaping information is being destroyed and only the buffer's memory
        // is preserved.
        let mut buffer = text::BufferLine::new(
            text,
            text::AttrsList::new(text::Attrs::new()),
            text::Shaping::Advanced,
        );
        let line = &buffer.layout(font_system, *font_size, f32::MAX, text::Wrap::None)[0];
        let line_glyphs = &line.glyphs;
        let placed = place_glyphs(line_glyphs);
        let metrics = LabelMetrics {
            max_ascent: line.max_ascent as u32,
            max_descent: line.max_descent as u32,
            width: line.w.ceil() as u32,
        };

        (metrics, placed)
    });

    let metrics = map_ref!(|metrics_and_placed_glyphs| metrics_and_placed_glyphs.0);

    Label {
        placed_glyphs: metrics_and_placed_glyphs,
        metrics,
    }
}

impl Label {
    // TODO: The shell here might be different compared to the one the label was created with.
    pub fn render(
        &self,
        shell: &Shell,
        matrix: Value<Matrix4>,
        surface_matrix: Value<Matrix4>,
    ) -> RenderLabel {
        let device = &shell.device;
        let queue = &shell.queue;
        let glyph_cache = &shell.glyph_cache;
        let font_system = &shell.font_system;
        let metrics_and_placed_glyphs = &self.placed_glyphs;

        // The pixel bounds in the center of the placed glyphs.
        let pixel_bounds = map_ref!(|metrics_and_placed_glyphs| {
            let metrics = &metrics_and_placed_glyphs.0;
            let (_, height) = metrics.size();
            // TODO: we might pull this up to the center of the part of the glyph above the
            // baseline.
            let half_height = height / 2;

            metrics_and_placed_glyphs
                .1
                .iter()
                .map(|positioned_glyph| {
                    let x = (positioned_glyph.hitbox_width as u32) / 2;
                    positioned_glyph.pixel_bounds_at((x, half_height))
                })
                .collect::<Vec<_>>()
        });

        let label_to_surface_matrix = map_ref!(|matrix, surface_matrix| surface_matrix * matrix);

        let glyph_classifications = map_ref!(|pixel_bounds, label_to_surface_matrix| {
            pixel_bounds
                .iter()
                .map(|pixel_bounds| {
                    let points = pixel_bounds
                        .to_rect()
                        .to_quad()
                        .map(|p| p.with_z(0.0))
                        .map(|p| label_to_surface_matrix.transform_point(p));
                    GlyphClassifier::from_transformed_pixel(&points)
                })
                .collect::<Vec<_>>()
        });

        // For now placements and textures have to be combined because we only receive placements
        // and the images together from the SwashCache, and the images are only accessible by
        // reference. TODO: Find a way to separate them.
        let placements_and_texture_views =
            map_ref!(|device,
                      queue,
                      font_system,
                      glyph_cache,
                      metrics_and_placed_glyphs,
                      glyph_classifications| {
                println!("Glyph classifications: {:?}", glyph_classifications);
                debug_assert!(metrics_and_placed_glyphs.1.len() == glyph_classifications.len());

                let mut font_system = font_system.borrow_mut();
                let mut glyph_cache = glyph_cache.borrow_mut();
                let glyph_cache = glyph_cache.deref_mut();

                let count = metrics_and_placed_glyphs.1.len();
                let mut r = Vec::with_capacity(count);

                for i in 0..count {
                    let placed_glyph = &metrics_and_placed_glyphs.1[i];
                    let glyph_classification = glyph_classifications[i];
                    let image = glyph_cache
                        .get_image(&mut font_system, placed_glyph.cache_key)
                        .as_ref();

                    if let Some(image) = image {
                        if image.placement.width != 0 && image.placement.height != 0 {
                            if let Ok(placement_and_texture_view) =
                                image_to_texture_with_classification(
                                    device,
                                    queue,
                                    image,
                                    glyph_classification,
                                )
                            {
                                r.push(Some(placement_and_texture_view));
                            } else {
                                r.push(None)
                            }
                        } else {
                            r.push(None)
                        }
                    } else {
                        r.push(None)
                    }
                }
                r
            });

        let vertex_buffers = map_ref!(
            |device, metrics_and_placed_glyphs, placements_and_texture_views| {
                let metrics = &metrics_and_placed_glyphs.0;
                placements_and_texture_views
                    .iter()
                    .enumerate()
                    .map(|(i, placement_and_view)| {
                        placement_and_view.as_ref().map(|(placement, _)| {
                            let rect = place_glyph(
                                metrics.max_ascent,
                                metrics_and_placed_glyphs.1[i].hitbox_pos,
                                *placement,
                            );

                            let vertices = glyph_to_texture_vertex((
                                rect.0.cast().unwrap(),
                                rect.1.cast().unwrap(),
                            ));

                            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("Vertex Buffer"),
                                contents: bytemuck::cast_slice(&vertices),
                                usage: wgpu::BufferUsages::VERTEX,
                            })
                        })
                    })
                    .collect::<Vec<_>>()
            }
        );

        RenderLabel {
            placements_and_texture_views,
            vertex_buffers,
        }
    }
}

/// A glyph that is positioned and ready to be rendered.
#[derive(Debug)]
pub struct PositionedGlyph {
    pub cache_key: text::CacheKey,
    pub hitbox_pos: (i32, i32),
    pub hitbox_width: f32,
}

impl PositionedGlyph {
    fn new(cache_key: text::CacheKey, hitbox_pos: (i32, i32), hitbox_width: f32) -> Self {
        Self {
            cache_key,
            hitbox_pos,
            hitbox_width,
        }
    }

    // The bounds enclosing a pixel at the offset of the hitbox
    fn pixel_bounds_at(&self, offset: (u32, u32)) -> Bounds {
        let x = self.hitbox_pos.0 + offset.0 as i32;
        let y = self.hitbox_pos.1 + offset.1 as i32;

        Bounds::new((x as f64, y as f64), ((x + 1) as f64, (y + 1) as f64))
    }
}

const RENDER_SUBPIXEL: bool = false;

fn place_glyphs(glyphs: &[text::LayoutGlyph]) -> Vec<PositionedGlyph> {
    glyphs
        .iter()
        .map(|glyph| {
            let fractional_pos = if RENDER_SUBPIXEL {
                (glyph.x, glyph.y)
            } else {
                (glyph.x.round(), glyph.y.round())
            };

            let (cc, x, y) = text::CacheKey::new(
                glyph.font_id,
                glyph.glyph_id,
                glyph.font_size,
                fractional_pos,
            );
            // Note: hitbox with is fractional, but does not change with / without subpixel
            // rendering.
            PositionedGlyph::new(cc, (x, y), glyph.w)
        })
        .collect()
}

fn image_to_texture_with_classification(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &SwashImage,
    classification: GlyphClassifier,
) -> Result<(text::Placement, PipelineTextureView)> {
    match classification {
        GlyphClassifier::Zoomed(_) | GlyphClassifier::PixelPerfect { .. } => {
            let padded = pad_image(image);
            Ok((
                padded.placement,
                PipelineTextureView::new(
                    Pipeline::Flat,
                    image_to_texture(device, queue, &padded),
                    (padded.placement.width, padded.placement.height),
                ),
            ))
        }
        GlyphClassifier::Distorted(_) => render_sdf(image)
            .map(|sdf_image| {
                (sdf_image.placement, {
                    PipelineTextureView::new(
                        Pipeline::Sdf,
                        image_to_texture(device, queue, &sdf_image),
                        (sdf_image.placement.width, sdf_image.placement.height),
                    )
                })
            })
            .ok_or_else(|| anyhow::anyhow!("Failed to generate SDF image")),
    }
}

/// Creates a texture and uploads the image's content to the GPU.
fn image_to_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &text::SwashImage,
) -> wgpu::TextureView {
    let texture_size = wgpu::Extent3d {
        width: image.placement.width,
        height: image.placement.height,
        depth_or_array_layers: 1,
    };

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        size: texture_size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        label: Some("Character Texture"),
        view_formats: &[],
    });

    // TODO: how to separate this from texture creation?
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &image.data,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(image.placement.width),
            rows_per_image: None,
        },
        texture_size,
    );

    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

// TODO: need a rect structure.

fn place_glyph(
    max_ascent: u32,
    hitbox_pos: (i32, i32),
    placement: text::Placement,
) -> (Point2<i32>, Point2<i32>) {
    let left = hitbox_pos.0 + placement.left;
    let top = hitbox_pos.1 + (max_ascent as i32) - placement.top;
    let right = left + placement.width as i32;
    let bottom = top + placement.height as i32;

    ((left, top).into(), (right, bottom).into())
}

fn glyph_to_texture_vertex(rect: (Point2<f32>, Point2<f32>)) -> [TextureVertex; 4] {
    let left = rect.0.x;
    let top = rect.0.y;
    let right = rect.1.x;
    let bottom = rect.1.y;

    [
        TextureVertex {
            position: (left, top, 0.0).into(),
            tex_coords: [0.0, 0.0],
        },
        TextureVertex {
            position: (left, bottom, 0.0).into(),
            tex_coords: [0.0, 1.0],
        },
        TextureVertex {
            position: (right, bottom, 0.0).into(),
            tex_coords: [1.0, 1.0],
        },
        TextureVertex {
            position: (right, top, 0.0).into(),
            tex_coords: [1.0, 0.0],
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GlyphClassifier {
    /// Pixel has the same size on screen compared to the rendered size (Zoomed(1.0))
    PixelPerfect { alignment: (bool, bool) },
    /// The center pixel is uniformly scaled by the following factor.
    Zoomed(f64),
    /// Either by some weird matrix, or perspective projection.
    Distorted(DistortedClassifier),
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq)]
enum DistortedClassifier {
    NonPlanar,
    NonRectangular,
    NonQuadratic,
}

/// For planar comparisons of Z values (we might transform them to pixels too, this way we can use
/// PIXEL_EPSILON).
const ULPS_Z: i64 = 8;
/// One thousandth of a pixel should be good enough.
const PIXEL_EPSILON: f64 = 0.0001;

impl GlyphClassifier {
    /// Classify the glyph based on a transformed pixel at the center of the glyph. `quad`
    /// represents the 4 points of the glyph in the final pixel coordinate system where `0,0` is the
    /// top left corner.
    ///
    /// The quad is clockwise, starting from the left top corner of the glyph as rendered.
    ///
    /// The 4 points are guaranteed to be in the same plane.
    pub fn from_transformed_pixel(quad: &[Point3; 4]) -> Self {
        // TODO: 3 Points might be enough.

        // TODO: may compare z for quad[3]?
        let planar_z = nearly_eq!(quad[0].z, quad[1].z, ulps = ULPS_Z)
            && nearly_eq!(quad[0].z, quad[2].z, ulps = ULPS_Z);

        if !planar_z {
            return GlyphClassifier::Distorted(DistortedClassifier::NonPlanar);
        }

        let rectangular = nearly_eq!(quad[0].y, quad[1].y, eps = PIXEL_EPSILON)
            && nearly_eq!(quad[2].y, quad[3].y, eps = PIXEL_EPSILON)
            && nearly_eq!(quad[0].x, quad[3].x, eps = PIXEL_EPSILON)
            && nearly_eq!(quad[1].x, quad[2].x, eps = PIXEL_EPSILON);

        if !rectangular {
            return GlyphClassifier::Distorted(DistortedClassifier::NonRectangular);
        }

        // TODO: may add the lower / or right parts of the rectangle and divide by 2.
        let scale_x = quad[1].x - quad[0].x;
        let scale_y = quad[2].y - quad[0].y;

        let quadratic = nearly_eq!(scale_x, scale_y, eps = PIXEL_EPSILON);
        if !quadratic {
            return GlyphClassifier::Distorted(DistortedClassifier::NonQuadratic);
        }

        let pixel_perfect = nearly_eq!(scale_x, 1.0, eps = PIXEL_EPSILON);
        if !pixel_perfect {
            return GlyphClassifier::Zoomed((scale_x + scale_y) / 2.0);
        }

        let aligned_x = nearly_eq!(quad[0].x, quad[0].x.floor(), eps = PIXEL_EPSILON);
        let aligned_y = nearly_eq!(quad[0].y, quad[0].y.floor(), eps = PIXEL_EPSILON);

        GlyphClassifier::PixelPerfect {
            alignment: (aligned_x, aligned_y),
        }
    }
}

fn render_sdf(image: &text::SwashImage) -> Option<text::SwashImage> {
    let width = image.placement.width as usize;
    let height = image.placement.height as usize;

    // TODO: Don't need the temporary SwashImage here.
    let padded_image = pad_image(image);
    let pad = DISTANCE_FIELD_PAD;
    let mut distance_field = vec![0u8; (width + 2 * pad) * (height + 2 * pad)];

    let sdf_ok = unsafe {
        generate_distance_field_from_image(
            distance_field.as_mut_slice(),
            &padded_image.data,
            width,
            height,
        )
    };

    if sdf_ok {
        return Some(text::SwashImage {
            placement: text::Placement {
                left: image.placement.left - pad as i32,
                top: image.placement.top + pad as i32,
                width: image.placement.width + 2 * pad as u32,
                height: image.placement.height + 2 * pad as u32,
            },
            data: distance_field,
            ..*image
        });
    };

    None
}

/// Pad an image by one pixel.
fn pad_image(image: &text::SwashImage) -> text::SwashImage {
    debug_assert!(image.content == text::SwashContent::Mask);
    let padded_data = pad_image_data(
        &image.data,
        image.placement.width as usize,
        image.placement.height as usize,
    );

    text::SwashImage {
        placement: text::Placement {
            left: image.placement.left - 1,
            top: image.placement.top + 1,
            width: image.placement.width + 2,
            height: image.placement.height + 2,
        },
        data: padded_data,
        ..*image
    }
}

fn pad_image_data(image: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut padded_image = vec![0u8; (width + 2) * (height + 2)];
    let row_offset = width + 2;
    for line in 0..height {
        let dest_offset = (line + 1) * row_offset + 1;
        let src_offset = line * width;
        padded_image[dest_offset..dest_offset + width]
            .copy_from_slice(&image[src_offset..src_offset + width]);
    }
    padded_image
}
