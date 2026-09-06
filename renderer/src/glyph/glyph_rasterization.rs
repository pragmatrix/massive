use parley::FontData;
use swash::FontRef;
use swash::scale::image::Image as SwashImage;
use swash::scale::{Render, ScaleContext, Source, StrikeWith, image::Content as SwashContent};
use swash::zeno::{Format, Placement};

use massive_geometry::ClipBoxPx;
use massive_shapes::GlyphKey;

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

/// Crop a rasterized glyph image to the crop window (`ClipBoxPx`).
///
/// The crop window is in the glyph's local pixel space (origin = advance origin, Y-up), the
/// same frame as the swash `Placement`. Finite edges clip; sentinel edges allow overflow.
/// Returns `None` if the crop is empty (the glyph is fully clipped).
fn crop_image(image: SwashImage, clip_box: &ClipBoxPx) -> Option<SwashImage> {
    let p = image.placement;
    // Ink box in Y-up space.
    let ink_left = p.left;
    let ink_right = p.left + p.width as i32;
    let ink_top = p.top;
    let ink_bottom = p.top - p.height as i32;

    // Clip box edges: `min` = (left, top), `max` = (right, bottom). Y-up so top > bottom.
    let clip_left = clip_box.min.x;
    let clip_right = clip_box.max.x;
    let clip_top = clip_box.min.y;
    let clip_bottom = clip_box.max.y;

    // Fast path: the crop window fully contains the ink box, so no cropping is needed. Return
    // the original image unchanged (no copy) — this is the common case (default multipliers,
    // overflow off).
    if clip_left <= ink_left
        && clip_right >= ink_right
        && clip_top >= ink_top
        && clip_bottom <= ink_bottom
    {
        return Some(image);
    }

    // Intersection of the ink box with the clip box.
    let crop_left = ink_left.max(clip_left);
    let crop_right = ink_right.min(clip_right);
    let crop_top = ink_top.min(clip_top);
    let crop_bottom = ink_bottom.max(clip_bottom);

    if crop_right <= crop_left || crop_top <= crop_bottom {
        return None;
    }

    let pixel_size = match image.content {
        SwashContent::Mask => 1,
        SwashContent::SubpixelMask => 4,
        SwashContent::Color => 4,
    };

    let src_width = p.width as usize;

    // Crop region in image coordinates (row 0 = top of image at Y = ink_top).
    let col_start = (crop_left - ink_left) as usize;
    let col_end = (crop_right - ink_left) as usize;
    let row_start = (ink_top - crop_top) as usize;
    let row_end = (ink_top - crop_bottom) as usize;

    let new_width = col_end - col_start;
    let new_height = row_end - row_start;

    let mut data = vec![0u8; new_width * new_height * pixel_size];
    for row in 0..new_height {
        let src_row = row_start + row;
        let src_offset = (src_row * src_width + col_start) * pixel_size;
        let dst_offset = row * new_width * pixel_size;
        data[dst_offset..dst_offset + new_width * pixel_size]
            .copy_from_slice(&image.data[src_offset..src_offset + new_width * pixel_size]);
    }

    Some(SwashImage {
        placement: Placement {
            left: crop_left,
            top: crop_top,
            width: new_width as u32,
            height: new_height as u32,
        },
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
    let font_ref = FontRef::from_index(font.data.as_ref(), font.index as usize)?;

    let mut scaler = context
        .builder(font_ref)
        .size(f32::from_bits(glyph_key.font_size_bits))
        .hint(param.hinted)
        // Detail: apply the weight variation for variable fonts.
        .variations(&[("wght", glyph_key.weight.0 as f32)])
        .build();

    // Select our source order
    Render::new(&[
        // Color outline with the first palette
        Source::ColorOutline(0),
        // Color bitmap with best fit selection mode
        Source::ColorBitmap(StrikeWith::BestFit),
        // Standard scalable outline
        Source::Outline,
    ])
    // Select a subpixel format
    .format(Format::Alpha)
    // Render the image
    .render(&mut scaler, glyph_key.glyph_id)
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
            placement: Placement {
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
        placement: Placement {
            left: image.placement.left - 1,
            top: image.placement.top + 1,
            width: image.placement.width + 2,
            height: image.placement.height + 2,
        },
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

    #[test]
    fn print_glyph_placement_test() {
        let bytes = include_bytes!("../../../../src/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf");
        let font_ref = FontRef::from_index(bytes, 0).unwrap();
        let mut context = ScaleContext::new();
        let mut scaler = context
            .builder(font_ref)
            .size(13.0)
            .hint(true)
            .build();

        for ch in ['a', 'b', 'c', 'd', 'e', 'g', 'j', 'p', 'q', 'y', 'A', 'M', 'W', 'l', '1', '/', '-', '_', '|', '.'] {
            let id = font_ref.charmap().map(ch);
            let img = Render::new(&[Source::Outline])
                .format(Format::Alpha)
                .render(&mut scaler, id)
                .unwrap();
            println!("char '{}' id {} placement: left={}, top={}, width={}, height={}",
                ch, id, img.placement.left, img.placement.top, img.placement.width, img.placement.height);
        }
    }
}

    #[test]
    fn print_terminal_font_metrics() {
        use parley::FontData;
        let bytes = include_bytes!("../../../../src/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf");
        let font_ref = FontRef::from_index(bytes, 0).unwrap();
        let m = font_ref.metrics(&[]);
        println!("ascent={}, descent={}, units_per_em={}", m.ascent, m.descent, m.units_per_em);
        let font_size = 13.0;
        let units_f = m.units_per_em as f32;
        let s = font_size / units_f;
        let asc = (m.ascent * s).trunc() as u32;
        let dsc = (m.descent * s).trunc() as u32;
        println!("asc_px={}, dsc_px={}, font_height={}", asc, dsc, asc + dsc);
    }

    #[test]
    fn print_clip_and_crop() {
        let bytes = include_bytes!("../../../../src/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf");
        let font_ref = FontRef::from_index(bytes, 0).unwrap();
        let mut context = ScaleContext::new();
        let mut scaler = context
            .builder(font_ref)
            .size(13.0)
            .hint(true)
            .build();

        // With default line_height 1.0, cell_width 1.0, font_height 16, ascender 13:
        // top = 13 + (16 - 16)/2 = 13, bottom = 13 - 16 = -3.
        // min = (0, 13), max = (8, -3).
        let clip_box = ClipBoxPx {
            min: PointPx::new(0, 13),
            max: PointPx::new(8, -3),
        };

        for ch in ['a', 'b', 'c', 'd', 'e', 'g', 'j', 'p', 'q', 'y', 'A', 'M', 'W', 'l', '1', '/', '-', '_', '|', '.'] {
            let id = font_ref.charmap().map(ch);
            let img = Render::new(&[Source::Outline])
                .format(Format::Alpha)
                .render(&mut scaler, id)
                .unwrap();
            let cropped = crop_image(img, &clip_box);
            println!("char '{}' cropped is_some={}", ch, cropped.is_some());
        }
    }
