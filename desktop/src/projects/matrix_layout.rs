//! The geometry of a project's matrix: slots on columns of one slot width and on rows as tall as
//! their tallest slot, each slot's content scaled to fit one slot extent and centered in its slot
//! (ADR 0023).

use std::array;
use std::cmp::max;
use std::collections::HashMap;

use euclid::vec2;
use massive_geometry::{Quaternion, Rect, RectPx, SizePx, Transform};
use massive_layout::{MeasuredLayout, Offset, Placement, Rect as LayoutRect, Size};

use crate::desktop_system::{DesktopLayoutState, DesktopTarget, LayoutContext, fullscreen_scale};
use crate::instance_presenter::instance_extent;
use crate::policy::constants::matrix;
use crate::projects::{MatrixRect, ProjectId, SlotContent, SlotPosition};

/// The extents of the columns and of the rows, indexed by axis like sizes and offsets, so both axes
/// share one measuring and centering rule.
type Extents = [Vec<u32>; 2];

/// The size the intervals span, spacing included. An empty matrix spans one slot extent, like an
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
    let extents = matrix_extents(
        &matrix_slots(project, context, children),
        slot_extent(context).width,
    );
    Size(array::from_fn(|axis| extents_span(&extents, axis)))
}

/// Places the slots. The intervals sit centered in the matrix's `size`, which exceeds their span,
/// the `measured_size`, when the project is stretched to its own slot, and each slot's content sits
/// centered in its slot, which exceeds the content when a band shrinks to fit.
pub fn place_matrix_children(
    project: ProjectId,
    context: &LayoutContext,
    size: Size<2>,
    measured_size: Size<2>,
    children: &[MeasuredLayout<2>],
    placements: &mut [Placement<Transform, 2>],
) {
    let slots = matrix_slots(project, context, children);
    let grid = MatrixGrid::new(&slots, context);
    let origin = intervals_origin(size, measured_size);

    for ((slot, measurement), placement) in slots.iter().zip(children).zip(placements) {
        let layout_size = expand_to_slot(slot, measurement, &grid.extents);
        let offset = Offset::from(array::from_fn(|axis| {
            let index = slot.indices()[axis];
            let presented = presented_extent(layout_size[axis], slot.scale);
            origin[axis]
                + grid.interval_start(axis, index)
                + centering_offset(grid.extents[axis][index], presented)
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

/// Where the columns and rows of `rect` in `project`'s matrix go, the spacing between them
/// included, in the coordinates of the layout root. A selection's outline goes there (ADR 0025).
/// `None` when `rect` reaches beyond the matrix's intervals.
pub fn matrix_area_placement(
    project: ProjectId,
    rect: MatrixRect,
    layout_state: &DesktopLayoutState,
    context: &LayoutContext,
) -> Option<Placement<Transform, 2>> {
    let matrix = DesktopTarget::matrix(project);
    let children: Vec<MeasuredLayout<2>> = context
        .hierarchy
        .get_nested(&matrix)
        .iter()
        .map(|child| layout_state.measured(child))
        .collect();
    let area = matrix_area(project, context, &children, rect)?;

    let matrix_placement = layout_state.absolute_placement(&matrix, context.hierarchy);
    let offset = intervals_origin(
        matrix_placement.rect.size,
        layout_state.measured(&matrix).size,
    ) + area.offset;
    let area_origin = DesktopLayoutState::origin_transform(&matrix_placement)
        * Transform::from_xy(offset[0] as f64, offset[1] as f64);
    Some(Placement::new(
        area_origin.pivoted_at(DesktopLayoutState::layout_local_center(area.size)),
        LayoutRect::new(matrix_placement.rect.offset + offset, area.size),
    ))
}

/// The rectangle that the columns and rows of `rect` cover, the spacing between them included,
/// relative to the start of the first interval. [`intervals_origin`] carries it into the matrix's
/// coordinates. Unassigned slots count like assigned ones, so this is the area a selection
/// outlines (ADR 0025). `None` when `rect` reaches beyond the matrix's intervals.
fn matrix_area(
    project: ProjectId,
    context: &LayoutContext,
    children: &[MeasuredLayout<2>],
    rect: MatrixRect,
) -> Option<LayoutRect<2>> {
    let grid = MatrixGrid::new(&matrix_slots(project, context, children), context);
    let top_left = SlotPosition::from(rect.min).indices();
    let bottom_right = SlotPosition::from(rect.max - vec2(1, 1)).indices();
    let mut start = [0; 2];
    let mut extent = [0; 2];
    for axis in 0..2 {
        let bottom_right_extent = *grid.extents[axis].get(bottom_right[axis])?;
        start[axis] = grid.interval_start(axis, top_left[axis]);
        let end = grid.interval_start(axis, bottom_right[axis]) + bottom_right_extent as i32;
        extent[axis] = (end - start[axis]) as u32;
    }
    Some(LayoutRect::new(start.into(), Size(extent)))
}

/// Where the intervals start in a matrix of `size`, so that they sit centered in it. The intervals
/// span `measured_size`, which `size` exceeds when the project is stretched to its own slot.
fn intervals_origin(size: Size<2>, measured_size: Size<2>) -> Offset<2> {
    Offset::from(array::from_fn(|axis| {
        centering_offset(size[axis], measured_size[axis])
    }))
}

/// Where a matrix's intervals lie relative to the first interval: their extents, and where each one
/// starts.
struct MatrixGrid {
    extents: Extents,
    starts: [Vec<i32>; 2],
}

impl MatrixGrid {
    /// The grid of the matrix that holds `slots`: each column's and row's extent, and where it
    /// starts relative to the first one.
    fn new(slots: &[MatrixSlot], context: &LayoutContext) -> Self {
        let extents = matrix_extents(slots, slot_extent(context).width);
        let starts =
            array::from_fn(|axis| interval_starts(&extents[axis], matrix::INTERVAL_SPACING[axis]));
        Self { extents, starts }
    }

    /// Where the interval at `index` along `axis` starts, relative to the first interval.
    fn interval_start(&self, axis: usize, index: usize) -> i32 {
        self.starts[axis][index]
    }
}

/// The extent of one slot, a launcher's default panel with its title bar: every column spans its
/// width, and no slot's content presents larger.
pub fn slot_extent(context: &LayoutContext) -> SizePx {
    instance_extent(context.default_panel_size, context.title_bar_metrics.height)
}

/// Where content spanning `span` starts to sit centered in `extent`, on whole pixels so it stays on
/// the pixel grid. A matrix's `extent` can lag its span for one pass
/// (`ChildPlacement::parent_size`).
fn centering_offset(extent: u32, span: u32) -> i32 {
    (extent.saturating_sub(span) / 2) as i32
}

/// Separate intrinsic and presented sizes keep child layout independent of the parent's intervals.
#[derive(Debug)]
struct MatrixSlot {
    layout_size: Size<2>,
    size: Size<2>,
    scale: f64,
    position: SlotPosition,
}

impl MatrixSlot {
    /// The slot's column and row, indexed by axis like the extents.
    fn indices(&self) -> [usize; 2] {
        self.position.indices()
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
    let positions: HashMap<SlotContent, SlotPosition> = context
        .configuration
        .project(project)
        .expect("a laid out project is configured")
        .slot_contents()
        .map(|(position, content)| (content, position))
        .collect();
    // ADR 0023: a parent-wide scale preserves the relative sizes of sibling projects.
    // Architecture: the shared scale is under discussion, see ADR 0023, "Under discussion".
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
            MatrixSlot {
                layout_size: measured,
                size: Size(array::from_fn(|axis| {
                    presented_extent(measured[axis], scale)
                })),
                scale,
                position: *positions
                    .get(&content)
                    .expect("slot content sits in a slot"),
            }
        })
        .collect()
}

/// The extent `layout` presents at `scale`, rounded outward so integer extents contain the full
/// fractional extent and content centered in an interval never overhangs it.
fn presented_extent(layout: u32, scale: f64) -> u32 {
    (layout as f64 * scale).ceil() as u32
}

/// ADR 0023: content shrinks to fit a slot's extent, like a full-screen instance fits the window,
/// so no slot widens its column or grows its row beyond the default size.
fn presentation_scale(slot_extent: SizePx, scene_size: SizePx) -> f64 {
    fullscreen_scale(slot_extent, scene_size).min(1.0)
}

/// The slot's layout size, stretched to its column's and row's intervals along the axes it expands.
fn expand_to_slot(
    slot: &MatrixSlot,
    measurement: &MeasuredLayout<2>,
    extents: &Extents,
) -> Size<2> {
    let indices = slot.indices();
    let mut size = slot.layout_size;
    for axis in 0..2 {
        if measurement.expandable_axes[axis] {
            // The extents are presented sizes. Flooring the unscaled extent keeps it inside the
            // interval.
            let extent = extents[axis][indices[axis]];
            size[axis] = max(size[axis], (extent as f64 / slot.scale).floor() as u32);
        }
    }
    size
}

/// The extents of a matrix's columns and rows: every column `column_width` regardless of its
/// slots, so columns line up across rows, and each row the tallest slot in it.
fn matrix_extents(slots: &[MatrixSlot], column_width: u32) -> Extents {
    let mut column_count = 0;
    let mut rows = Vec::new();
    for slot in slots {
        let [column, row] = slot.indices();
        column_count = max(column_count, column + 1);
        widen_extent(&mut rows, row, slot.size[1]);
    }
    [vec![column_width; column_count], rows]
}

/// Grows the extents to reach `index`. A row no slot reaches keeps a zero extent, but its spacing.
fn widen_extent(extents: &mut Vec<u32>, index: usize, extent: u32) {
    if extents.len() <= index {
        extents.resize(index + 1, 0);
    }
    extents[index] = max(extents[index], extent);
}

/// The extent the intervals along `axis` cover, the spacing between them included: the matrix's
/// measured size along it.
fn extents_span(extents: &Extents, axis: usize) -> u32 {
    let extents = &extents[axis];
    extents.iter().sum::<u32>()
        + matrix::INTERVAL_SPACING[axis] * extents.len().saturating_sub(1) as u32
}

/// Where each interval starts, after its predecessors and their spacing.
fn interval_starts(extents: &[u32], spacing: u32) -> Vec<i32> {
    extents
        .iter()
        .scan(0, |start, extent| {
            let current = *start;
            *start += (extent + spacing) as i32;
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
            position: SlotPosition { column, row },
        };
        let slots = [slot(Size([50, 40]), 0, 0), slot(Size([400, 70]), 2, 0)];

        let [columns, rows] = matrix_extents(&slots, 300);
        assert_eq!(columns, [300, 300, 300]);
        assert_eq!(rows, [70]);
    }

    #[test]
    fn slots_stretch_only_along_their_expandable_axes() {
        let layout_size = Size([100, 50]);
        let slot = MatrixSlot {
            layout_size,
            size: layout_size,
            scale: 1.0,
            position: SlotPosition { column: 1, row: 1 },
        };
        let extents: Extents = [vec![500, 200], vec![400, 300]];

        let vertical = MeasuredLayout::new(layout_size, [false, true]);
        assert_eq!(expand_to_slot(&slot, &vertical, &extents), Size([100, 300]));
        let both = MeasuredLayout::new(layout_size, [true, true]);
        assert_eq!(expand_to_slot(&slot, &both, &extents), Size([200, 300]));
        assert_eq!(
            expand_to_slot(&slot, &layout_size.into(), &extents),
            layout_size
        );
    }

    /// A scaled slot stretches in layout space, so its presented extent fills the interval.
    #[test]
    fn scaled_slots_stretch_to_the_unscaled_interval() {
        let layout_size = Size([100, 50]);
        let slot = MatrixSlot {
            layout_size,
            size: Size([50, 25]),
            scale: 0.5,
            position: SlotPosition { column: 0, row: 0 },
        };
        let extents: Extents = [vec![201], vec![301]];

        let both = MeasuredLayout::new(layout_size, [true, true]);
        assert_eq!(expand_to_slot(&slot, &both, &extents), Size([402, 602]));
    }
}
