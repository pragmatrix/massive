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
/// defaults' own metadata). Clusters keep their line positions; only the run boundaries change.
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
        let width = match remaining.get(group_len) {
            Some(next) => next.x - first.x,
            None => run.width - first.x,
        };
        runs.push(shaped_run_to_glyph_run(
            &ShapedRun {
                clusters: remaining[..group_len].to_vec(),
                max_ascent: run.max_ascent,
                max_descent: run.max_descent,
                width,
                engine: run.shaping_engine,
            },
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
    use massive_shapes::{FontManager, ShapingEngineKind};

    /// Regression: `shape_text` used to stamp one hard-coded color on the whole line, so
    /// attributed ranges (terminal logs, syntax highlighting) rendered in a single color.
    /// Every attribute segment must reach its runs as their `text_color`/`text_weight`.
    #[test]
    fn shape_text_splits_runs_per_attribute() {
        let fonts = FontManager::bare(ShapingEngineKind::Parley).with_font(JETBRAINS_MONO);
        let mut shaper = fonts.shaper();

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
        let mut shaper = fonts.shaper();

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
}
