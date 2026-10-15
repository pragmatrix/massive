//! The geometry of a project's matrix: slots on column and row tracks, each track as large as its
//! largest slot, nested projects shown at a shared presentation scale (ADR 0015).

use std::cmp::max;

use massive_geometry::{Quaternion, Rect, RectPx, SizePx, Transform};
use massive_layout::{MeasuredLayout, Offset, Placement, Rect as LayoutRect, Size};

use crate::desktop_system::LayoutContext;
use crate::instance_presenter::instance_extent;
use crate::projects::{MatrixPlacement, ProjectId, SlotContent};

const MATRIX_COLUMN_SPACING: u32 = 10;
const MATRIX_ROW_SPACING: u32 = 10;

/// The size the tracks span, column and row spacing included. An empty matrix spans one instance
/// extent, like an empty launcher, so an empty project stays as readable as one holding a launcher.
pub fn measure_matrix(
    project: ProjectId,
    context: &LayoutContext,
    children: &[MeasuredLayout<2>],
) -> Size<2> {
    if children.is_empty() {
        return instance_extent(context.default_panel_size, context.title_bar_metrics.height)
            .into();
    }
    // Shared presented extents keep measured matrix bounds consistent with placement.
    let (columns, rows) = matrix_tracks(matrix_slots(project, context, children));
    [
        tracks_span(&columns, MATRIX_COLUMN_SPACING),
        tracks_span(&rows, MATRIX_ROW_SPACING),
    ]
    .into()
}

/// Places the slots on their tracks, the tracks vertically centered in the matrix's `size`, which
/// exceeds their span when the project is stretched to its own slot.
pub fn place_matrix_children(
    project: ProjectId,
    context: &LayoutContext,
    size: Size<2>,
    children: &[MeasuredLayout<2>],
    placements: &mut [Placement<Transform, 2>],
) {
    let slots = matrix_slots(project, context, children);
    // Every slot sizes the tracks before any slot can be placed, so the slots are produced twice.
    let (columns, rows) = matrix_tracks(slots.clone());
    // Whole pixels keep the slots' content on the pixel grid.
    let top = (size[1].saturating_sub(tracks_span(&rows, MATRIX_ROW_SPACING)) / 2) as i32;

    for ((slot, measurement), placement) in slots.zip(children).zip(placements) {
        let offset = Offset::from([
            track_offset(
                &columns,
                slot.placement.column as usize,
                MATRIX_COLUMN_SPACING,
            ),
            top + track_offset(&rows, slot.placement.row as usize, MATRIX_ROW_SPACING),
        ]);
        let layout_size = expand_to_slot(&slot, measurement, [&columns, &rows]);
        let rect: RectPx = LayoutRect::new(offset, layout_size).into();
        let rect = Rect::from(rect);
        // The exact scaled center preserves left alignment despite rounded track extents.
        let center = rect.origin() + (rect.size() * slot.scale).center();
        let transform = Transform::new(center.with_z(0.0), Quaternion::IDENTITY, slot.scale);
        // Intrinsic rectangles keep presentation scaling from resizing the child scene.
        *placement = Placement::new(transform, LayoutRect::new(offset, layout_size));
    }
}

/// Separate intrinsic and presented sizes keep child layout independent of parent track allocation.
#[derive(Debug)]
struct MatrixSlot {
    layout_size: Size<2>,
    size: Size<2>,
    scale: f64,
    placement: MatrixPlacement,
}

/// Cached intrinsic measurements keep parent scaling independent of child layout.
fn matrix_slots<'a>(
    project: ProjectId,
    context: &'a LayoutContext,
    children: &'a [MeasuredLayout<2>],
) -> impl Iterator<Item = MatrixSlot> + Clone + 'a {
    let contents = context.hierarchy.matrix_slots(project);
    // ADR 0015: a parent-wide scale preserves the relative sizes of sibling projects.
    let widest_project = contents
        .clone()
        .zip(children)
        .filter_map(|(content, child)| {
            matches!(content, SlotContent::Project(_)).then_some(child.size[0])
        })
        .max()
        .unwrap_or(0);
    let project_scale = presentation_scale(context.default_panel_size.width, widest_project);

    contents.zip(children).map(move |(content, child)| {
        let measured = child.size;
        let scale = match content {
            SlotContent::Launcher(_) => 1.0,
            SlotContent::Project(_) => project_scale,
        };
        let size = SizePx::new(measured[0], measured[1]).to_f64();
        // Round outward so integer tracks contain the full fractional presented extent.
        let size = (size * scale).ceil().to_u32();
        MatrixSlot {
            layout_size: measured,
            size: size.into(),
            scale,
            placement: context
                .configuration
                .placement_of_content(project, content)
                .expect("slot content has a matrix placement"),
        }
    })
}

/// ADR 0015: width-only shrinking preserves readability without penalizing tall projects.
fn presentation_scale(preferred_width: u32, scene_width: u32) -> f64 {
    if scene_width <= preferred_width {
        return 1.0;
    }
    preferred_width as f64 / scene_width as f64
}

/// The slot's layout size, stretched to its column's and row's tracks along the axes it expands.
fn expand_to_slot(
    slot: &MatrixSlot,
    measurement: &MeasuredLayout<2>,
    tracks: [&[u32]; 2],
) -> Size<2> {
    let indices = [slot.placement.column, slot.placement.row];
    let mut size = slot.layout_size;
    for axis in 0..2 {
        if measurement.expandable_axes[axis] {
            // Tracks hold presented extents; flooring the unscaled extent keeps it inside the track.
            let track = tracks[axis][indices[axis] as usize];
            size[axis] = max(size[axis], (track as f64 / slot.scale).floor() as u32);
        }
    }
    size
}

/// The column and row tracks of a matrix: each track the largest slot in it.
fn matrix_tracks(slots: impl Iterator<Item = MatrixSlot>) -> (Vec<u32>, Vec<u32>) {
    let mut columns = Vec::new();
    let mut rows = Vec::new();
    for slot in slots {
        widen_track(&mut columns, slot.placement.column as usize, slot.size[0]);
        widen_track(&mut rows, slot.placement.row as usize, slot.size[1]);
    }
    (columns, rows)
}

/// Grows the tracks to reach `index`; a track no slot reaches keeps a zero extent but its spacing.
fn widen_track(tracks: &mut Vec<u32>, index: usize, extent: u32) {
    if tracks.len() <= index {
        tracks.resize(index + 1, 0);
    }
    tracks[index] = max(tracks[index], extent);
}

fn tracks_span(tracks: &[u32], spacing: u32) -> u32 {
    tracks.iter().sum::<u32>() + spacing * tracks.len().saturating_sub(1) as u32
}

fn track_offset(tracks: &[u32], index: usize, spacing: u32) -> i32 {
    tracks
        .iter()
        .take(index)
        .map(|track| *track as i32 + spacing as i32)
        .sum()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::OrderedHierarchy;
    use crate::instance_presenter::InstanceTitleBarMetrics;
    use crate::projects::persistence::parse_configuration;

    /// An empty project keeps an instance's extent, so it stays readable without siblings.
    #[test]
    fn an_empty_matrix_spans_one_instance_extent() {
        let configuration =
            parse_configuration(Path::new("/config/desktop.json"), r#"{ "slots": [] }"#).unwrap();
        let hierarchy = OrderedHierarchy::default();
        let context = LayoutContext {
            hierarchy: &hierarchy,
            configuration: &configuration,
            default_panel_size: SizePx::new(800, 600),
            window_size: SizePx::new(800, 600),
            focused_instance: None,
            title_bar_metrics: InstanceTitleBarMetrics::from_scale_factor(1.0),
        };
        assert_eq!(
            measure_matrix(ProjectId::new(), &context, &[]),
            instance_extent(context.default_panel_size, context.title_bar_metrics.height).into()
        );
    }

    #[test]
    fn presentation_scale_fits_width_without_enlarging() {
        assert_eq!(presentation_scale(300, 600), 0.5);
        assert_eq!(presentation_scale(300, 300), 1.0);
        assert_eq!(presentation_scale(300, 150), 1.0);
    }

    #[test]
    fn presentation_scale_of_an_empty_scene_is_one() {
        assert_eq!(presentation_scale(300, 0), 1.0);
    }

    #[test]
    fn slots_stretch_only_along_their_expandable_axes() {
        let layout_size = Size([100, 50]);
        let slot = MatrixSlot {
            layout_size,
            size: layout_size,
            scale: 1.0,
            placement: MatrixPlacement { column: 1, row: 1 },
        };
        let tracks: [&[u32]; 2] = [&[500, 200], &[400, 300]];

        let vertical = MeasuredLayout::new(layout_size, [false, true]);
        assert_eq!(expand_to_slot(&slot, &vertical, tracks), Size([100, 300]));
        let both = MeasuredLayout::new(layout_size, [true, true]);
        assert_eq!(expand_to_slot(&slot, &both, tracks), Size([200, 300]));
        assert_eq!(
            expand_to_slot(&slot, &layout_size.into(), tracks),
            layout_size
        );
    }

    /// A scaled slot stretches in layout space, so its presented extent fills the track.
    #[test]
    fn scaled_slots_stretch_to_the_unscaled_track() {
        let layout_size = Size([100, 50]);
        let slot = MatrixSlot {
            layout_size,
            size: Size([50, 25]),
            scale: 0.5,
            placement: MatrixPlacement { column: 0, row: 0 },
        };
        let tracks: [&[u32]; 2] = [&[201], &[301]];

        let both = MeasuredLayout::new(layout_size, [true, true]);
        assert_eq!(expand_to_slot(&slot, &both, tracks), Size([402, 602]));
    }
}
