#![allow(dead_code)]
use approx::{abs_diff_eq, ulps_eq};
use massive_geometry::Vector3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GlyphClass {
    /// Pixel has the same size on screen compared to the rendered size (Zoomed(1.0))
    PixelPerfect { alignment: (bool, bool) },
    /// The center pixel is uniformly scaled by the following factor.
    Zoomed(f64),
    /// Either by some weird matrix, or perspective projection.
    Distorted(DistortionClass),
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DistortionClass {
    NonPlanar,
    NonRectangular,
    NonQuadratic,
}

/// For planar comparisons of Z values (we might transform them to pixels too, this way we can use
/// PIXEL_EPSILON).
const ULPS_Z: u32 = 8;
/// A ten-thousandth of a pixel should be good enough.
const PIXEL_EPSILON: f64 = 0.0001;

impl GlyphClass {
    /// Classify the glyph based on a transformed pixel to the surface pixel range at the center of
    /// the glyph. `quad` represents the 4 points of the glyph in the final pixel coordinate system
    /// where `0,0` is the top left corner.
    ///
    /// The quad is clockwise, starting from the left top corner of the glyph as rendered.
    ///
    /// The 4 points are guaranteed to be in the same plane.
    pub fn from_transformed_pixel(quad: &[Vector3; 4]) -> Self {
        // TODO: 3 Points might be enough.

        // TODO: may compare z for quad[3]?
        let planar_z = ulps_eq!(quad[0].z, quad[1].z, max_ulps = ULPS_Z)
            && ulps_eq!(quad[0].z, quad[2].z, max_ulps = ULPS_Z);

        if !planar_z {
            return GlyphClass::Distorted(DistortionClass::NonPlanar);
        }

        let rectangular = pixel_eq(quad[0].y, quad[1].y)
            && pixel_eq(quad[2].y, quad[3].y)
            && pixel_eq(quad[0].x, quad[3].x)
            && pixel_eq(quad[1].x, quad[2].x);

        if !rectangular {
            return GlyphClass::Distorted(DistortionClass::NonRectangular);
        }

        // TODO: may add the lower / or right parts of the rectangle and divide by 2.
        let scale_x = quad[1].x - quad[0].x;
        let scale_y = quad[2].y - quad[0].y;

        let quadratic = pixel_eq(scale_x, scale_y);
        if !quadratic {
            return GlyphClass::Distorted(DistortionClass::NonQuadratic);
        }

        let pixel_perfect = pixel_eq(scale_x, 1.0);
        if !pixel_perfect {
            return GlyphClass::Zoomed((scale_x + scale_y) / 2.0);
        }

        let aligned_x = pixel_eq(quad[0].x, quad[0].x.floor());
        let aligned_y = pixel_eq(quad[0].y, quad[0].y.floor());

        GlyphClass::PixelPerfect {
            alignment: (aligned_x, aligned_y),
        }
    }
}

fn pixel_eq(a: f64, b: f64) -> bool {
    abs_diff_eq!(a, b, epsilon = PIXEL_EPSILON)
}
