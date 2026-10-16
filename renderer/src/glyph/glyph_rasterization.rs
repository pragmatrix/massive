use swash::FontRef;
use swash::scale::image::Image as SwashImage;
use swash::scale::{Render, ScaleContext, Source, StrikeWith, image::Content as SwashContent};
use swash::zeno::{Angle, Format, Placement, Transform};

use massive_geometry::{BoxPx, PointPx};
use massive_shapes::FontData;
use massive_shapes::{ClipBoxPx, GlyphKey};

use super::SwashRasterizationParam;
use super::distance_field_gen::{DISTANCE_FIELD_PAD, generate_distance_field_from_image};
use crate::glyph::GlyphRasterizationParam;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RasterizedGlyphKey {
    pub glyph: GlyphKey,
    pub param: GlyphRasterizationParam,
}

/// Rasterize a glyph into [`SwashImage`] as either monochrome, colored, or SDF, with appropriate
/// padding prepared to be used as a texture.
///
/// TODO: Using this for SDF and non-SDF glyphs may duplicate rasterization of the non-sdf
/// [`SwashImage`]s that  are the basis for the SDF generation.
pub fn rasterize_glyph_with_padding(
    font: &FontData,
    context: &mut ScaleContext,
    key: &RasterizedGlyphKey,
) -> Option<SwashImage> {
    let param = key.param;
    let without_padding = rasterize_glyph(font, context, key.glyph, param.swash)?;

    // Crop the bitmap to the glyph's crop window (finite edges clip, sentinel edges allow
    // overflow) before any padding/SDF. The crop is part of the glyph's rasterization identity,
    // so a glyph cropped differently is a different bitmap and a different cache entry. A
    // fully-clipped glyph (empty crop) is treated as empty and not rendered.
    let cropped = crop_image(without_padding, &key.glyph.clip_box)?;
    if cropped.content == SwashContent::Mask && param.prefer_sdf {
        // SDF rendering adds its own padding.
        return render_sdf(&cropped);
    }

    // Add a one pixel padding to make this work with texture mapping.
    Some(pad_image(&cropped))
}

/// The ink box of a rasterized image in its local pixel space: origin = advance origin,
/// Y-up, so `min` = (left, bottom), `max` = (right, top). The swash `Placement` is Y-down
/// (`left`/`top` are the ink box's top-left, size extends downward).
fn placement_to_ink_box(placement: Placement) -> BoxPx {
    let left = placement.left;
    let top = placement.top;
    let width = placement.width as i32;
    let height = placement.height as i32;
    BoxPx::new(
        PointPx::new(left, top - height),
        PointPx::new(left + width, top),
    )
}

/// The inverse of [`placement_to_ink_box`]. The box must be non-empty with positive size.
fn ink_box_to_placement(b: BoxPx) -> Placement {
    let size = b.size().cast::<u32>();
    Placement {
        left: b.min.x,
        top: b.max.y,
        width: size.width,
        height: size.height,
    }
}

/// Crop a rasterized glyph image to the crop window (`ClipBoxPx`).
///
/// The crop window is in the glyph's local pixel space (origin = advance origin, Y-up), the
/// same frame as the swash `Placement`. Finite edges clip; sentinel edges allow overflow.
/// Returns `None` if the ink is empty or the crop is empty (the glyph is fully clipped).
fn crop_image(image: SwashImage, clip_box: &ClipBoxPx) -> Option<SwashImage> {
    let ink = placement_to_ink_box(image.placement);

    // A zero-size ink has no content bytes to crop or pad; treat it as empty.
    if ink.is_empty() {
        return None;
    }

    // Intersection of the ink box with the clip box; `None` = fully clipped.
    let crop = ink.intersection(clip_box)?;

    // Fast path: the crop window fully contains the ink box, so no cropping is needed. Return
    // the original image unchanged (no copy) — this is the common case (default multipliers,
    // overflow off).
    if crop == ink {
        return Some(image);
    }

    // Crop region in image coordinates (row 0 = top of image at Y = ink max.y).
    let pixel_size = match image.content {
        SwashContent::Mask => 1,
        SwashContent::SubpixelMask => 4,
        SwashContent::Color => 4,
    };
    let src_width = image.placement.width as usize;
    let col_start = (crop.min.x - ink.min.x) as usize;
    let row_start = (ink.max.y - crop.max.y) as usize;
    let crop_size = crop.size().cast::<u32>();
    let new_width = crop_size.width as usize;
    let new_height = crop_size.height as usize;

    let mut data = vec![0u8; new_width * new_height * pixel_size];
    for row in 0..new_height {
        let src_row = row_start + row;
        let src_offset = (src_row * src_width + col_start) * pixel_size;
        let dst_offset = row * new_width * pixel_size;
        data[dst_offset..dst_offset + new_width * pixel_size]
            .copy_from_slice(&image.data[src_offset..src_offset + new_width * pixel_size]);
    }

    Some(SwashImage {
        placement: ink_box_to_placement(crop),
        data,
        ..image
    })
}

/// Rasterize a glyph using a swash scaler built from a [`FontData`].
pub fn rasterize_glyph(
    font: &FontData,
    context: &mut ScaleContext,
    glyph_key: GlyphKey,
    param: SwashRasterizationParam,
) -> Option<SwashImage> {
    let font_ref = FontRef::from_index(font.data.as_ref().as_ref(), font.index as usize)?;

    let mut scaler = context
        .builder(font_ref)
        .size(f32::from_bits(glyph_key.font_size_bits))
        .hint(param.hinted)
        // Detail: apply the weight variation for variable fonts.
        .variations(&[("wght", glyph_key.weight.0 as f32)])
        .build();

    // Select our source order
    let image = Render::new(&[
        // Color outline with the first palette
        Source::ColorOutline(0),
        // Color bitmap with best fit selection mode
        Source::ColorBitmap(StrikeWith::BestFit),
        // Standard scalable outline
        Source::Outline,
    ])
    // Select a subpixel format
    .format(Format::Alpha)
    .transform(glyph_key.synthetic_italic.then(synthetic_italic_skew))
    // Render the image
    .render(&mut scaler, glyph_key.glyph_id)?;

    // swash transforms outlines only; bitmap glyphs (Apple Color Emoji) come back upright.
    let is_bitmap = matches!(image.source, Source::Bitmap(_) | Source::ColorBitmap(_));
    Some(if glyph_key.synthetic_italic && is_bitmap {
        shear_bitmap(&image)
    } else {
        image
    })
}

/// The slant for a face without italic: 14°, the angle both shaping engines assume (cosmic-text
/// renders it, fontique suggests it), so either engine's synthetic italic looks the same.
const SYNTHETIC_ITALIC_DEGREES: f32 = 14.0;

fn synthetic_italic_skew() -> Transform {
    Transform::skew(
        Angle::from_degrees(SYNTHETIC_ITALIC_DEGREES),
        Angle::from_degrees(0.0),
    )
}

/// Slants a bitmap glyph like [`synthetic_italic_skew`] slants outlines: each row shifts right by
/// its height above the baseline, linearly resampled. Color pixels are blended premultiplied so
/// transparent neighbours don't darken the edges, and stay straight alpha as the atlas expects.
fn shear_bitmap(image: &SwashImage) -> SwashImage {
    let Placement {
        left,
        top,
        width,
        height,
    } = image.placement;
    if width == 0 || height == 0 {
        return image.clone();
    }
    let channels = match image.content {
        SwashContent::Color => 4,
        SwashContent::Mask | SwashContent::SubpixelMask => 1,
    };
    let slant = SYNTHETIC_ITALIC_DEGREES.to_radians().tan();
    // Row centers are Y-up from the baseline: row 0 is the top row.
    let shift = |row: u32| (top as f32 - row as f32 - 0.5) * slant;
    let new_left = left + shift(height - 1).floor() as i32;
    let new_width = (left + width as i32 + shift(0).ceil() as i32 + 1 - new_left) as u32;

    let mut data = vec![0u8; (new_width * height) as usize * channels];
    for row in 0..height {
        let source = &image.data[(row * width) as usize * channels..][..width as usize * channels];
        let target =
            &mut data[(row * new_width) as usize * channels..][..new_width as usize * channels];
        // Source pixel coordinate sampled by target column 0.
        let origin = (new_left - left) as f32 - shift(row);
        for (column, pixel) in target.chunks_exact_mut(channels).enumerate() {
            let position = origin + column as f32;
            let index = position.floor();
            let fraction = position - index;
            let a = premultiplied(source, channels, index as i64);
            let b = premultiplied(source, channels, index as i64 + 1);
            let mixed: [f32; 4] = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * fraction);
            write_straight(pixel, mixed);
        }
    }

    SwashImage {
        source: image.source,
        content: image.content,
        placement: Placement {
            left: new_left,
            top,
            width: new_width,
            height,
        },
        data,
    }
}

/// A row pixel as premultiplied RGBA (masks use the first component); outside the row is clear.
fn premultiplied(row: &[u8], channels: usize, index: i64) -> [f32; 4] {
    let Some(pixel) = usize::try_from(index)
        .ok()
        .and_then(|index| row.get(index * channels..(index + 1) * channels))
    else {
        return [0.0; 4];
    };
    match *pixel {
        [r, g, b, a] => {
            let alpha = a as f32 / 255.0;
            [
                r as f32 * alpha,
                g as f32 * alpha,
                b as f32 * alpha,
                a as f32,
            ]
        }
        [value] => [value as f32, 0.0, 0.0, 0.0],
        _ => unreachable!("pixels are RGBA or single-channel masks"),
    }
}

/// Writes a blended pixel back in the image's straight-alpha layout.
fn write_straight(pixel: &mut [u8], [r, g, b, a]: [f32; 4]) {
    match pixel {
        [pr, pg, pb, pa] => {
            let alpha = a / 255.0;
            let straight = |c: f32| {
                if alpha > 0.0 {
                    (c / alpha).round() as u8
                } else {
                    0
                }
            };
            (*pr, *pg, *pb, *pa) = (straight(r), straight(g), straight(b), a.round() as u8);
        }
        [value] => *value = r.round() as u8,
        _ => unreachable!("pixels are RGBA or single-channel masks"),
    }
}

pub fn render_sdf(image: &SwashImage) -> Option<SwashImage> {
    let width = image.placement.width as usize;
    let height = image.placement.height as usize;

    // This one pixel padding is solely for the input of the `generate_distance_field_from_image``.
    // The resulting image does not include the input padding, only the output padding
    // [`DISTANCE_FIELD_PAD`].
    // Therefore, the padded image's placement is _not_ taken into account.
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
        return Some(SwashImage {
            // Padding expands the ink box symmetrically: SDF distance is measured from the
            // ink edges, so the placement grows by `pad` on every side.
            placement: ink_box_to_placement(
                placement_to_ink_box(image.placement)
                    .inflate(DISTANCE_FIELD_PAD as i32, DISTANCE_FIELD_PAD as i32),
            ),
            data: distance_field,
            ..*image
        });
    };

    None
}

/// Pad an image by one pixel.
pub fn pad_image(image: &SwashImage) -> SwashImage {
    let pixel_size = match image.content {
        SwashContent::Mask => 1,
        SwashContent::SubpixelMask => 4,
        SwashContent::Color => 4,
    };

    let padded_data = pad_image_data(
        &image.data,
        image.placement.width as usize,
        image.placement.height as usize,
        pixel_size,
    );

    SwashImage {
        // Padding expands the ink box symmetrically by one pixel on every side (Y-up).
        placement: ink_box_to_placement(placement_to_ink_box(image.placement).inflate(1, 1)),
        data: padded_data,
        ..*image
    }
}

fn pad_image_data(image: &[u8], width: usize, height: usize, pixel_size: usize) -> Vec<u8> {
    let mut padded_image = vec![0u8; (width + 2) * (height + 2) * pixel_size];
    let src_line_size = width * pixel_size;
    let dst_line_size = (width + 2) * pixel_size;
    for line in 0..height {
        let dest_offset = (line + 1) * dst_line_size + pixel_size;
        let src_offset = line * src_line_size;
        padded_image[dest_offset..dest_offset + src_line_size]
            .copy_from_slice(&image[src_offset..src_offset + src_line_size]);
    }
    padded_image
}

#[cfg(test)]
mod tests {
    use super::*;
    use massive_geometry::PointPx;

    /// A sentinel edge (`i32::MIN`/`i32::MAX`) means "no clip on that side". `UNCLIPPED` must
    /// return the image unchanged, and a clip box with overflow on one axis must still crop the
    /// other axis rather than being treated as fully clipped.
    #[test]
    fn crop_image_handles_sentinel_edges() {
        let bytes = include_bytes!(
            "../../../assets/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf"
        );
        let font_ref = FontRef::from_index(bytes, 0).unwrap();
        let mut context = ScaleContext::new();
        let mut scaler = context.builder(font_ref).size(13.0).hint(true).build();

        let id = font_ref.charmap().map('a');
        let img = Render::new(&[Source::Outline])
            .format(Format::Alpha)
            .render(&mut scaler, id)
            .unwrap();
        let original = img.clone();

        // No clipping on any edge: the image is returned unchanged.
        let unclipped = crop_image(img.clone(), &ClipBoxPx::UNCLIPPED).unwrap();
        assert_eq!(unclipped.placement.left, original.placement.left);
        assert_eq!(unclipped.placement.top, original.placement.top);
        assert_eq!(unclipped.placement.width, original.placement.width);
        assert_eq!(unclipped.placement.height, original.placement.height);
        assert_eq!(unclipped.data, original.data);

        // Overflow on the vertical axis only: the horizontal edges still clip, so the result
        // is narrower than the original but not empty. Sentinels are per-axis and uniform in
        // the newtype: `i32::MIN`/`i32::MAX` = "no clip" on that side (Y-up, so vertical
        // overflow is min.y = MIN, max.y = MAX).
        let overflow_v = ClipBoxPx::from(BoxPx::new(
            PointPx::new(0, i32::MIN),
            PointPx::new(4, i32::MAX),
        ));
        let cropped = crop_image(img.clone(), &overflow_v).unwrap();
        assert!(cropped.placement.width < original.placement.width);
        assert!(cropped.placement.width > 0);

        // A fully-clipped glyph (empty crop) is treated as empty.
        let empty = ClipBoxPx::from(BoxPx::new(PointPx::new(100, -200), PointPx::new(200, -100)));
        assert!(crop_image(img.clone(), &empty).is_none());

        // A zero-size ink box has no content bytes: it is empty regardless of the clip box.
        let zero_ink = SwashImage {
            placement: Placement {
                left: 3,
                top: 5,
                width: 0,
                height: 0,
            },
            ..img
        };
        assert!(crop_image(zero_ink, &ClipBoxPx::UNCLIPPED).is_none());
    }

    /// Padding expands the ink box by one pixel on every side; the placement round-trip must
    /// match the previous scalar arithmetic exactly (Y-down `top` grows by 1).
    #[test]
    fn pad_image_inflates_placement() {
        let placement = Placement {
            left: -3,
            top: 9,
            width: 7,
            height: 4,
        };
        let padded = pad_image(&SwashImage {
            placement,
            data: vec![0; (placement.width * placement.height) as usize],
            ..SwashImage::default()
        });
        assert_eq!(padded.placement.left, placement.left - 1);
        assert_eq!(padded.placement.top, placement.top + 1);
        assert_eq!(padded.placement.width, placement.width + 2);
        assert_eq!(padded.placement.height, placement.height + 2);
    }

    /// Bitmap glyphs are slanted by resampling: a vertical bar must lean right (top rows shifted
    /// further than bottom rows) while every row keeps its height above the baseline.
    #[test]
    fn shear_bitmap_leans_a_vertical_bar_right() {
        let height = 20;
        let image = SwashImage {
            source: Source::ColorBitmap(StrikeWith::BestFit),
            content: SwashContent::Color,
            placement: Placement {
                left: 0,
                top: 16,
                width: 1,
                height,
            },
            data: [255, 0, 0, 255].repeat(height as usize),
        };
        let sheared = shear_bitmap(&image);
        assert_eq!(sheared.placement.top, image.placement.top);
        assert_eq!(sheared.placement.height, height);

        // The ink center of a row, in absolute x.
        let ink_center = |row: u32| {
            let width = sheared.placement.width as usize;
            let pixels = &sheared.data[row as usize * width * 4..][..width * 4];
            let (sum, weight) = pixels.as_chunks::<4>().0.iter().enumerate().fold(
                (0.0, 0.0),
                |(sum, weight), (x, pixel)| {
                    let alpha = pixel[3] as f32;
                    (sum + x as f32 * alpha, weight + alpha)
                },
            );
            assert_eq!(
                pixels.as_chunks::<4>().0.iter().find(|p| p[3] > 0).unwrap()[0],
                255
            );
            sheared.placement.left as f32 + sum / weight
        };
        let lean = ink_center(0) - ink_center(height - 1);
        let expected = (height - 1) as f32 * SYNTHETIC_ITALIC_DEGREES.to_radians().tan();
        assert!(
            (lean - expected).abs() < 0.1,
            "lean {lean}, expected {expected}"
        );
    }
}
