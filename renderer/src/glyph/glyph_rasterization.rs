use cosmic_text as text;
use swash::{
    scale::{Render, ScaleContext, Source, StrikeWith},
    zeno::{Format, Vector},
};
use text::SwashContent;

use super::{
    distance_field_gen::{generate_distance_field_from_image, DISTANCE_FIELD_PAD},
    RasterizedGlyphKey, SwashRasterizationParam,
};

/// Rasterize a glyph into [`SwashImage`] as either monochrome, colored, or SDF, with appropriate
/// padding prepared to be used as a texture.
///
/// TODO: Using this for SDF and non-SDF glyphs may duplicate rasterization of the non-sdf
/// [`SwashImage`]s that  are the basis for the SDF generation.
pub fn rasterize_glyph_with_padding(
    font_system: &mut text::FontSystem,
    context: &mut ScaleContext,
    key: &RasterizedGlyphKey,
) -> Option<text::SwashImage> {
    let param = key.param;
    let without_padding = rasterize_glyph(font_system, context, key.text, param.swash)?;
    if without_padding.content == SwashContent::Mask && param.prefer_sdf {
        // SDF rendering adds its own padding.
        return render_sdf(&without_padding);
    }

    // Add a one pixel padding to make this compatible with texture mapping.
    Some(pad_image(&without_padding))
}

pub fn rasterize_glyph(
    font_system: &mut text::FontSystem,
    context: &mut ScaleContext,
    cache_key: text::CacheKey,
    param: SwashRasterizationParam,
) -> Option<text::SwashImage> {
    // Copied from cosmic_text/swash.rs, because we might need finer control and don't need a cache.
    // TODO: Find a way to prevent excessive locking of the font system here. Note that it needs to
    // be mutable for font caching (can we implement our own)

    let font = match font_system.get_font(cache_key.font_id) {
        Some(some) => some,
        None => {
            log::warn!("did not find font {:?}", cache_key.font_id);
            return None;
        }
    };

    // Build the scaler
    let mut scaler = context
        .builder(font.as_swash())
        .size(f32::from_bits(cache_key.font_size_bits))
        .hint(param.hinted)
        .variations(&[("wght", param.weight.0 as f32)])
        .build();

    // Compute the fractional offset -- you'll likely want to quantize this
    // in a real renderer
    //
    // TODO: Is this used? It seems that it's only relevant for subpixel rendering.
    let offset = Vector::new(cache_key.x_bin.as_float(), cache_key.y_bin.as_float());

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
    // Apply the fractional offset
    .offset(offset)
    // Render the image
    .render(&mut scaler, cache_key.glyph_id)
}

pub fn render_sdf(image: &text::SwashImage) -> Option<text::SwashImage> {
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
pub fn pad_image(image: &text::SwashImage) -> text::SwashImage {
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
