//! The geometry of a project's matrix: slots on columns of one slot width and on rows as tall as
//! their tallest slot, each slot's content scaled to fit one slot extent and centered in its slot
//! (ADR 0023).

use std::array;
use std::cmp::max;
use std::collections::HashMap;

use massive_geometry::{Quaternion, Rect, RectPx, SizePx, Transform};
use massive_layout::{MeasuredLayout, Offset, Placement, Rect as LayoutRect, Size};

use crate::desktop_system::{LayoutContext, fullscreen_scale};
use crate::instance_presenter::instance_extent;
use crate::projects::{MatrixPlacement, ProjectId, SlotContent};

/// The spacing between columns and between rows.
const TRACK_SPACING: [u32; 2] = [10, 10];

/// The extents of the columns and of the rows, indexed by axis like sizes and offsets, so both axes
/// share one measuring and centering rule.
type Tracks = [Vec<u32>; 2];

/// The size the tracks span, spacing included. An empty matrix spans one instance extent, like an
/// empty launcher, so an empty project stays as readable as one holding a launcher.
pub fn measure_matrix(
    project: ProjectId,
    context: &LayoutContext,
    children: &[MeasuredLayout<2>],
) -> Size<2> {
    if children.is_empty() {
        return slot_extent(context).into();
    }
    // Shared presented extents keep measured matrix bounds consistent with placement.
    let tracks = matrix_tracks(
        &matrix_slots(project, context, children),
        slot_extent(context).width,
    );
    Size(array::from_fn(|axis| tracks_span(&tracks, axis)))
}

/// Places the slots. The tracks sit centered in the matrix's `size`, which exceeds their span when
/// the project is stretched to its own slot, and each slot's content sits centered in its slot,
/// which exceeds the content when a band shrinks to fit.
pub fn place_matrix_children(
    project: ProjectId,
    context: &LayoutContext,
    size: Size<2>,
    children: &[MeasuredLayout<2>],
    placements: &mut [Placement<Transform, 2>],
) {
    let slots = matrix_slots(project, context, children);
    let tracks = matrix_tracks(&slots, slot_extent(context).width);
    let origin: [i32; 2] =
        array::from_fn(|axis| centering_offset(size[axis], tracks_span(&tracks, axis)));
    let starts: [Vec<i32>; 2] =
        array::from_fn(|axis| track_starts(&tracks[axis], TRACK_SPACING[axis]));

    for ((slot, measurement), placement) in slots.iter().zip(children).zip(placements) {
        let layout_size = expand_to_slot(slot, measurement, &tracks);
        let offset = Offset::from(array::from_fn(|axis| {
            let index = slot.indices()[axis];
            let presented = (layout_size[axis] as f64 * slot.scale).ceil() as u32;
            origin[axis] + starts[axis][index] + centering_offset(tracks[axis][index], presented)
        }));
        let rect: RectPx = LayoutRect::new(offset, layout_size).into();
        let rect = Rect::from(rect);
        // The exact scaled center keeps the slot's corner at its offset despite rounded extents.
        let center = rect.origin() + (rect.size() * slot.scale).center();
        let transform = Transform::new(center.with_z(0.0), Quaternion::IDENTITY, slot.scale);
        // Intrinsic rectangles keep presentation scaling from resizing the child scene.
        *placement = Placement::new(transform, LayoutRect::new(offset, layout_size));
    }
}

/// The extent of one slot, a launcher's default panel with its title bar: every column spans its
/// width, and no slot's content presents larger.
fn slot_extent(context: &LayoutContext) -> SizePx {
    instance_extent(context.default_panel_size, context.title_bar_metrics.height)
}

/// Where content spanning `span` starts to sit centered in `extent`, on whole pixels so it stays on
/// the pixel grid. A re-measured matrix is placed once at its previous size before its project
/// re-places it, so `extent` can lag the span.
fn centering_offset(extent: u32, span: u32) -> i32 {
    (extent.saturating_sub(span) / 2) as i32
}

/// Separate intrinsic and presented sizes keep child layout independent of parent track allocation.
#[derive(Debug)]
struct MatrixSlot {
    layout_size: Size<2>,
    size: Size<2>,
    scale: f64,
    placement: MatrixPlacement,
}

impl MatrixSlot {
    /// The slot's column and row, indexed by axis like its tracks.
    fn indices(&self) -> [usize; 2] {
        [self.placement.column as usize, self.placement.row as usize]
    }
}

/// Cached intrinsic measurements keep parent scaling independent of child layout.
fn matrix_slots(
    project: ProjectId,
    context: &LayoutContext,
    children: &[MeasuredLayout<2>],
) -> Vec<MatrixSlot> {
    let contents = context.hierarchy.matrix_slots(project);
    // One lookup table keeps placing a matrix linear in its slot count.
    let placements: HashMap<SlotContent, MatrixPlacement> = context
        .configuration
        .project(project)
        .expect("a laid out project is configured")
        .slots()
        .iter()
        .map(|slot| (slot.content(), slot.placement))
        .collect();
    // ADR 0023: a parent-wide scale preserves the relative sizes of sibling projects.
    let largest_project = contents
        .clone()
        .zip(children)
        .filter_map(|(content, child)| {
            matches!(content, SlotContent::Project(_)).then_some(child.size)
        })
        .fold(SizePx::zero(), |largest, size| {
            SizePx::new(max(largest.width, size[0]), max(largest.height, size[1]))
        });
    let slot_extent = slot_extent(context);
    let project_scale = presentation_scale(slot_extent, largest_project);

    contents
        .zip(children)
        .map(|(content, child)| {
            let measured = child.size;
            let scale = match content {
                // A visor or an empty launcher is one slot extent; only a band shrinks.
                SlotContent::Launcher(_) => {
                    presentation_scale(slot_extent, SizePx::new(measured[0], measured[1]))
                }
                SlotContent::Project(_) => project_scale,
            };
            let size = SizePx::new(measured[0], measured[1]).to_f64();
            // Round outward so integer tracks contain the full fractional presented extent.
            let size = (size * scale).ceil().to_u32();
            MatrixSlot {
                layout_size: measured,
                size: size.into(),
                scale,
                placement: *placements
                    .get(&content)
                    .expect("slot content has a matrix placement"),
            }
        })
        .collect()
}

/// ADR 0023: content shrinks to fit a slot's extent, like a full-screen instance fits the window,
/// so no slot widens its column or grows its row beyond the default size.
fn presentation_scale(slot_extent: SizePx, scene_size: SizePx) -> f64 {
    fullscreen_scale(slot_extent, scene_size).min(1.0)
}

/// The slot's layout size, stretched to its column's and row's tracks along the axes it expands.
fn expand_to_slot(slot: &MatrixSlot, measurement: &MeasuredLayout<2>, tracks: &Tracks) -> Size<2> {
    let indices = slot.indices();
    let mut size = slot.layout_size;
    for axis in 0..2 {
        if measurement.expandable_axes[axis] {
            // Tracks hold presented extents; flooring the unscaled extent keeps it inside the track.
            let track = tracks[axis][indices[axis]];
            size[axis] = max(size[axis], (track as f64 / slot.scale).floor() as u32);
        }
    }
    size
}

/// The tracks of a matrix: every column `column_width` regardless of its slots, so columns line up
/// across rows, and each row the tallest slot in it.
fn matrix_tracks(slots: &[MatrixSlot], column_width: u32) -> Tracks {
    let mut column_count = 0;
    let mut rows = Vec::new();
    for slot in slots {
        let [column, row] = slot.indices();
        column_count = max(column_count, column + 1);
        widen_track(&mut rows, row, slot.size[1]);
    }
    [vec![column_width; column_count], rows]
}

/// Grows the tracks to reach `index`; a track no slot reaches keeps a zero extent but its spacing.
fn widen_track(tracks: &mut Vec<u32>, index: usize, extent: u32) {
    if tracks.len() <= index {
        tracks.resize(index + 1, 0);
    }
    tracks[index] = max(tracks[index], extent);
}

fn tracks_span(tracks: &Tracks, axis: usize) -> u32 {
    let tracks = &tracks[axis];
    tracks.iter().sum::<u32>() + TRACK_SPACING[axis] * tracks.len().saturating_sub(1) as u32
}

/// Where each track starts, after its predecessors and their spacing.
fn track_starts(tracks: &[u32], spacing: u32) -> Vec<i32> {
    tracks
        .iter()
        .scan(0, |start, track| {
            let current = *start;
            *start += (track + spacing) as i32;
            Some(current)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_scale_fits_both_axes_without_enlarging() {
        let slot = SizePx::new(300, 200);
        assert_eq!(presentation_scale(slot, SizePx::new(600, 200)), 0.5);
        assert_eq!(presentation_scale(slot, SizePx::new(300, 800)), 0.25);
        assert_eq!(presentation_scale(slot, SizePx::new(150, 100)), 1.0);
    }

    /// Columns line up across rows, so no slot widens its column and a column without slots keeps
    /// its width.
    #[test]
    fn every_column_spans_the_slot_width_regardless_of_its_slots() {
        let slot = |size: Size<2>, column, row| MatrixSlot {
            layout_size: size,
            size,
            scale: 1.0,
            placement: MatrixPlacement { column, row },
        };
        let slots = [slot(Size([50, 40]), 0, 0), slot(Size([400, 70]), 2, 0)];

        let [columns, rows] = matrix_tracks(&slots, 300);
        assert_eq!(columns, [300, 300, 300]);
        assert_eq!(rows, [70]);
    }

    #[test]
    fn presentation_scale_of_an_empty_scene_is_one() {
        assert_eq!(
            presentation_scale(SizePx::new(300, 200), SizePx::zero()),
            1.0
        );
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
        let tracks: Tracks = [vec![500, 200], vec![400, 300]];

        let vertical = MeasuredLayout::new(layout_size, [false, true]);
        assert_eq!(expand_to_slot(&slot, &vertical, &tracks), Size([100, 300]));
        let both = MeasuredLayout::new(layout_size, [true, true]);
        assert_eq!(expand_to_slot(&slot, &both, &tracks), Size([200, 300]));
        assert_eq!(
            expand_to_slot(&slot, &layout_size.into(), &tracks),
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
        let tracks: Tracks = [vec![201], vec![301]];

        let both = MeasuredLayout::new(layout_size, [true, true]);
        assert_eq!(expand_to_slot(&slot, &both, &tracks), Size([402, 602]));
    }
}
