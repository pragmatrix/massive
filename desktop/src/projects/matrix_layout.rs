//! The geometry of a project's matrix: slots on column and row tracks, each track as large as its
//! largest slot, nested projects shown at a shared presentation scale (ADR 0015).

use std::cmp::max;

use massive_geometry::{Quaternion, Rect, RectPx, SizePx, Transform};
use massive_layout::{MeasuredLayout, Offset, Placement, Rect as LayoutRect, Size};

use crate::desktop_system::LayoutContext;
use crate::projects::{MatrixPlacement, ProjectId, SlotContent};

const MATRIX_COLUMN_SPACING: u32 = 10;
const MATRIX_ROW_SPACING: u32 = 10;

/// The size the tracks span, column and row spacing included.
pub fn measure_matrix(
    project: ProjectId,
    context: &LayoutContext,
    children: &[MeasuredLayout<2>],
) -> Size<2> {
    // Shared presented extents keep measured matrix bounds consistent with placement.
    let (columns, rows) = matrix_tracks(matrix_slots(project, context, children));
    [
        tracks_span(&columns, MATRIX_COLUMN_SPACING),
        tracks_span(&rows, MATRIX_ROW_SPACING),
    ]
    .into()
}

pub fn place_matrix_children(
    project: ProjectId,
    context: &LayoutContext,
    children: &[MeasuredLayout<2>],
    placements: &mut [Placement<Transform, 2>],
) {
    let slots = matrix_slots(project, context, children);
    // Every slot sizes the tracks before any slot can be placed, so the slots are produced twice.
    let (columns, rows) = matrix_tracks(slots.clone());

    for ((slot, measurement), placement) in slots.zip(children).zip(placements) {
        let offset = Offset::from([
            track_offset(
                &columns,
                slot.placement.column as usize,
                MATRIX_COLUMN_SPACING,
            ),
            track_offset(&rows, slot.placement.row as usize, MATRIX_ROW_SPACING),
        ]);
        let layout_size = expand_to_row(&slot, measurement, &rows);
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

/// The slot's layout size, its height stretched to its row's track when it expands vertically.
fn expand_to_row(slot: &MatrixSlot, measurement: &MeasuredLayout<2>, rows: &[u32]) -> Size<2> {
    let mut size = slot.layout_size;
    if measurement.expandable_axes[1] {
        // Tracks hold presented extents; only unscaled launchers expand, so they equal layout ones.
        assert_eq!(slot.scale, 1.0, "only unscaled slots expand");
        size[1] = max(size[1], rows[slot.placement.row as usize]);
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
    use super::*;

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
    fn only_vertically_expandable_slots_stretch_to_their_row() {
        let layout_size = Size([100, 50]);
        let slot = MatrixSlot {
            layout_size,
            size: layout_size,
            scale: 1.0,
            placement: MatrixPlacement { column: 0, row: 1 },
        };
        let rows = [400, 300];

        let expandable = MeasuredLayout::new(layout_size, [false, true]);
        assert_eq!(expand_to_row(&slot, &expandable, &rows), Size([100, 300]));
        assert_eq!(
            expand_to_row(&slot, &layout_size.into(), &rows),
            layout_size
        );
    }
}
