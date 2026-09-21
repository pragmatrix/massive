//! Multi-line attributed text shaping on the engine-neutral shaping contract.
//!
//! Each line is shaped separately and positioned on `line_height` steps, matching the legacy
//! cosmic-text behavior; each returned [`GlyphRun`] carries the color of the attribute covering
//! its text span.

use std::ops::Range;

use serde::{Deserialize, Serialize};
use serde_tuple::{Deserialize_tuple, Serialize_tuple};

use massive_geometry::{Color, Vector3};

use massive_shapes::{
    GlyphRun, ShapedRun, Shaper, ShapingRequest, TextAttributes, TextFamily, TextWeight,
    shaped_run_to_glyph_run,
};

/// A serializable representation of highlighted code.
#[derive(Debug, Serialize, Deserialize)]
pub struct AttributedText {
    pub text: String,
    pub attributes: Vec<TextAttribute>,
}

#[derive(Debug, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct TextAttribute {
    pub range: Range<usize>,
    pub color: Color,
    pub weight: TextWeight,
}

/// Shape `text` into [`GlyphRun`]s, one run per attribute segment per line, honoring
/// per-attribute weights/colors.
///
/// The text is split into lines (byte offsets stay aligned with the original text), each line's
/// attribute ranges are re-based locally, and each shaped run is translated down by `line_height`
/// per line index. The returned height covers all lines.
pub fn shape_text(
    shaper: &mut Shaper<'_>,
    text: &str,
    attributes: &[TextAttribute],
    font_size: f32,
    line_height: f32,
    translation: impl Into<Option<Vector3>>,
) -> (Vec<GlyphRun>, f64) {
    let translation = translation.into().unwrap_or(Vector3::new(0., 0., 0.));

    let mut runs = Vec::new();
    let mut height: f64 = 0.;

    for (index, (line_offset, line_text)) in syntax::split_lines(text).enumerate() {
        let default_attributes = TextAttributes::default()
            .with_family(TextFamily::Monospace)
            .with_weight(TextWeight::NORMAL);
        let mut request =
            ShapingRequest::new(line_text, default_attributes.clone()).with_metadata();
        // Re-map the attribute ranges covering this line onto the line's local byte range,
        // tagging each with its caller identity so the echoed cluster metadata resolves back
        // to the attribute (identities start at 1 to stay distinct from the default
        // attributes' metadata; ranges need not cover the whole line — uncovered text takes
        // the default attributes).
        for (identity, ta) in attributes.iter().enumerate() {
            let start = ta.range.start.saturating_sub(line_offset);
            let end = ta
                .range
                .end
                .saturating_sub(line_offset)
                .min(line_text.len());
            if start >= end {
                continue;
            }
            request.ranges.push((
                start..end,
                TextAttributes::default()
                    .with_weight(ta.weight)
                    .with_color(ta.color)
                    .with_metadata(identity + 1),
            ));
        }

        let line_top = index as f64 * line_height as f64;
        let line_translation = translation + Vector3::new(0., line_top, 0.);
        if let Some(run) = shaper.shape(&request, font_size) {
            runs.extend(attribute_runs(
                &run,
                &request.ranges,
                &default_attributes,
                line_translation,
            ));
        }
        height = height.max(line_top + line_height as f64);
    }

    (runs, height)
}

/// Partition one shaped line into [`GlyphRun`]s, one per contiguous attribute segment: the run
/// carries the attributes covering its clusters' text.
///
/// The engines echoed each request range's metadata onto its clusters (the metadata mechanism
/// is enabled by `shape_text`), so grouping by cluster metadata resolves the attribute per
/// segment with no byte-range probing: the echoed value resolves to the range carrying that
/// identity, falling back to the default attributes when none does (uncovered text echoes the
/// defaults' own metadata). Clusters keep their line positions and their visual `x`, only the
/// run boundaries and widths change: each run's width is the sum of its clusters' advances,
/// which is direction-independent under the engines' visual RTL placement (non-monotonic
/// `x`).
fn attribute_runs<'a>(
    run: &ShapedRun,
    ranges: &'a [(Range<usize>, TextAttributes<'a>)],
    default_attributes: &'a TextAttributes<'a>,
    translation: Vector3,
) -> Vec<GlyphRun> {
    let attributes_at = |metadata: usize| -> &'a TextAttributes<'a> {
        ranges
            .iter()
            .find(|(_, attributes)| attributes.metadata == metadata)
            .map(|(_, attributes)| attributes)
            .unwrap_or(default_attributes)
    };

    // Echoed identities must resolve to exactly one attribute set: duplicate range identities
    // make the lookup multi-valued, and an identity colliding with the default's steals the
    // fallback. Fail loudly in debug instead of silently coloring from an arbitrary match.
    debug_assert_eq!(
        ranges.len(),
        ranges
            .iter()
            .map(|(_, attributes)| attributes.metadata)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        "request ranges carry duplicate metadata identities"
    );
    debug_assert!(
        ranges
            .iter()
            .all(|(_, attributes)| attributes.metadata != default_attributes.metadata),
        "a request range's metadata collides with the default attributes' metadata"
    );

    let mut runs = Vec::new();
    let mut remaining = run.clusters.as_slice();
    while let Some(first) = remaining.first() {
        let attributes = attributes_at(first.metadata);
        // The group ends at the first cluster carrying a different attribute, or at the line's
        // end.
        let group_len = remaining
            .iter()
            .position(|c| c.metadata != first.metadata)
            .unwrap_or(remaining.len());
        // The group's width is its total advance, not `next.x − first.x`: engines position
        // RTL clusters visually, so `x` is not monotonic in cluster order and origin
        // differences measure backwards (they also miss trailing whitespace/left-bearing
        // when a group lands at the line's end). The advance is direction-independent.
        let width = remaining[..group_len].iter().map(|c| c.advance).sum();
        runs.push(shaped_run_to_glyph_run(
            run,
            &remaining[..group_len],
            width,
            attributes.color,
            attributes.weight,
            translation,
        ));
        remaining = &remaining[group_len..];
    }
    runs
}

mod syntax {
    /// Split `text` into lines as `(byte_offset, line)` keeping the trailing `\n` on its line so
    /// per-attribute byte ranges re-base without remapping.
    pub fn split_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
        let mut rest = Some(text);
        let mut offset = 0;
        std::iter::from_fn(move || {
            let current = rest?;
            if current.is_empty() {
                rest = None;
                return None;
            }
            match current.find('\n') {
                Some(pos) => {
                    rest = Some(&current[pos + 1..]);
                    let line = &current[..=pos];
                    let start = offset;
                    offset += line.len();
                    Some((start, line))
                }
                None => {
                    rest = None;
                    let start = offset;
                    offset += current.len();
                    Some((start, current))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::JETBRAINS_MONO;
    use massive_shapes::{FaceId, FontManager, ShapedCluster, ShapedGlyph, ShapingEngineKind};

    /// Regression: `shape_text` used to stamp one hard-coded color on the whole line, so
    /// attributed ranges (terminal logs, syntax highlighting) rendered in a single color.
    /// Every attribute segment must reach its runs as their `text_color`/`text_weight`.
    #[test]
    fn shape_text_splits_runs_per_attribute() {
        let fonts = FontManager::bare(ShapingEngineKind::Parley).with_font(JETBRAINS_MONO);
        let context = fonts.new_shaping_context();
        let mut shaper = context.shaper();

        let red = Color::rgb(1.0, 0.0, 0.0);
        let attributes = vec![
            TextAttribute {
                range: 0..2,
                color: red,
                weight: TextWeight::NORMAL,
            },
            TextAttribute {
                range: 2..4,
                color: Color::BLACK,
                weight: TextWeight::BOLD,
            },
        ];
        let (runs, _) = shape_text(&mut shaper, "abcd", &attributes, 32., 40., None);

        assert_eq!(runs.len(), 2, "one run per attribute segment");
        assert_eq!(runs[0].text_color, red);
        assert_eq!(runs[0].text_weight, TextWeight::NORMAL);
        assert_eq!(runs[1].text_color, Color::BLACK);
        assert_eq!(runs[1].text_weight, TextWeight::BOLD);
    }

    /// Ranges need not cover the whole text: uncovered bytes shape under the default
    /// attributes and group into their own default-colored run.
    #[test]
    fn shape_text_default_run_between_ranges() {
        let fonts = FontManager::bare(ShapingEngineKind::Parley).with_font(JETBRAINS_MONO);
        let context = fonts.new_shaping_context();
        let mut shaper = context.shaper();

        let red = Color::rgb(1.0, 0.0, 0.0);
        let blue = Color::rgb(0.0, 0.0, 1.0);
        let attributes = vec![
            TextAttribute {
                range: 0..1,
                color: red,
                weight: TextWeight::NORMAL,
            },
            TextAttribute {
                range: 3..4,
                color: blue,
                weight: TextWeight::BOLD,
            },
        ];
        let (runs, _) = shape_text(&mut shaper, "abcd", &attributes, 32., 40., None);

        // One run per attribute segment, and the gap carries the defaults.
        assert_eq!(runs.len(), 3, "red | default gap | blue");
        assert_eq!(runs[0].text_color, red);
        assert_eq!(
            runs[1].text_color,
            Color::BLACK,
            "the uncovered gap is default-colored"
        );
        assert_eq!(runs[1].text_weight, TextWeight::NORMAL);
        assert_eq!(runs[2].text_color, blue);
        assert_eq!(runs[2].text_weight, TextWeight::BOLD);
    }

    /// Regression: a group's width must be the total advance of its clusters, not
    /// `next.x − first.x`.
    ///
    /// `attribute_runs` assumed walking `run.clusters` in order yields monotonically
    /// increasing x. Parley's `Run::clusters()` iterates in *logical* order while clusters
    /// carry their *visual* line positions: inside an RTL run the logically-earlier
    /// cluster sits at the larger x, so differencing across a group boundary measures
    /// backwards (`next.x − first.x < 0` → saturates to width 0) and the final group's
    /// `run.width − first.x` measures from the group's visually-rightmost origin to the
    /// line's end — neither is the group's advance. The group's cluster-advance sum is
    /// direction-independent and stays correct under both encodings.
    ///
    /// The engine-level companion
    /// (`font_manager::tests::rtl_runs_position_clusters_visually` in massive-shapes,
    /// currently `#[ignore]`d) pins the visual RTL positioning that makes this encoding
    /// real; both engines render RTL mirrored today (see its doc comment), and fixing
    /// that will make engines' stored `x` non-monotonic in list order, at which point
    /// this test's premise becomes real engine output rather than a synthetic
    /// construction.
    #[test]
    fn attribute_runs_rtl_group_width_is_direction_independent() {
        // "سلامa" shaped in an RTL paragraph: clusters stored in logical order (parley's
        // native encoding) while x holds the visual origin. The RTL run occupies the line's
        // right half (each cluster advancing 5px, right-to-left), the LTR island "ab" the
        // left half (10px per cluster); the line advance is 40px.
        let mut glyphs = Vec::new();
        let mut clusters = Vec::new();
        // (byte range, visual origin, metadata)
        for (index, (byte_range, x, metadata)) in [
            (0..2, 35.0, 1),  // س — logically first, visually rightmost
            (2..4, 30.0, 1),  // ل
            (4..6, 25.0, 1),  // ا
            (6..8, 20.0, 1),  // م — visually leftmost of the RTL run
            (8..9, 0.0, 0),   // a — the LTR island precedes it visually
            (9..10, 10.0, 0), // b
        ]
        .into_iter()
        .enumerate()
        {
            glyphs.push(ShapedGlyph {
                glyph_id: 1,
                face_id: FaceId::new(0),
                font_size: 16.0,
                weight: TextWeight::NORMAL,
                x: 0.0,
                y: 0.0,
            });
            clusters.push(ShapedCluster {
                byte_range,
                x,
                // RTL clusters advance backwards on the line; the attribute-covered
                // run carries 4 such clusters, the trailing LTR one 2 clusters @ 10px.
                advance: if metadata == 1 { 5.0 } else { 10.0 },
                glyph_range: index as u32..index as u32 + 1,
                metadata,
            });
        }
        let run = ShapedRun {
            clusters,
            glyphs,
            max_ascent: 14.0,
            max_descent: 5.0,
            width: 40.0,
            engine: ShapingEngineKind::Parley,
        };

        let attributed = TextAttributes::default()
            .with_color(Color::rgb(1.0, 0.0, 0.0))
            .with_metadata(1);
        let ranges = [(0..8, attributed)];
        let default_attributes = TextAttributes::default();

        let runs = attribute_runs(&run, &ranges, &default_attributes, Vector3::ZERO);

        // One run per attribute segment, RTL group first (logical order).
        assert_eq!(runs.len(), 2, "one run per attribute segment");
        assert_eq!(runs[0].text_color, Color::rgb(1.0, 0.0, 0.0));
        // Each group's width is its clusters' advance sum (4×5px RTL, 2×10px LTR) — not
        // `next.x − first.x = 0 − 35 = −35` → 0 for the RTL group, nor
        // `run.width − first.x = 40` for the trailing group.
        assert_eq!(
            runs[0].metrics.width, 20,
            "RTL group width = its advance sum"
        );
        assert_eq!(
            runs[1].metrics.width, 20,
            "trailing group width = its advance sum, not run.width − first.x"
        );
    }
}
