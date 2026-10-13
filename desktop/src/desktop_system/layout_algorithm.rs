use std::cmp::max;

use derive_more::From;

use massive_geometry::{Quaternion, Rect, RectPx, SizePx, Transform, Vector3};
use massive_layout::{
    LayoutAlgorithm, LayoutAxis, MeasuredLayout, Offset, Placement, Rect as LayoutRect, Size,
    Thickness,
};

use super::{Aggregates, DesktopTarget, LayoutContext, ProjectPart, TargetLayout};
use crate::layout::{ContainerBuilder, ToContainer};
use crate::projects::{MatrixPlacement, ProjectId, SlotContent};

const PROJECT_PADDING: u32 = 10;
const PROJECT_HEADER_MIN_HEIGHT: u32 = 24;
const PROJECT_HEADER_SPACING: u32 = 10;
const MATRIX_COLUMN_SPACING: u32 = 10;
const MATRIX_ROW_SPACING: u32 = 10;

#[derive(Debug, From)]
enum LayoutSpec {
    Container {
        axis: LayoutAxis,
        padding: Thickness<2>,
        spacing: u32,
    },
    #[from]
    Leaf(SizePx),
}

impl From<LayoutAxis> for LayoutSpec {
    fn from(axis: LayoutAxis) -> Self {
        Self::Container {
            axis,
            padding: Default::default(),
            spacing: 0,
        }
    }
}

impl From<ContainerBuilder> for LayoutSpec {
    fn from(value: ContainerBuilder) -> Self {
        let (axis, padding, spacing) = value.into_parts();
        LayoutSpec::Container {
            axis,
            padding,
            spacing,
        }
    }
}

pub struct DesktopLayoutAlgorithm<'a> {
    pub aggregates: &'a Aggregates,
    pub context: LayoutContext<'a>,
}

impl LayoutAlgorithm<DesktopTarget, Transform, 2> for DesktopLayoutAlgorithm<'_> {
    fn place_children(
        &self,
        id: &DesktopTarget,
        parent_size: Size<2>,
        child_measurements: &[MeasuredLayout<2>],
        placements: &mut [Placement<Transform, 2>],
    ) {
        assert_eq!(
            child_measurements.len(),
            placements.len(),
            "one placement slot per child"
        );
        match id {
            DesktopTarget::Launcher(launcher) => self.aggregates.launchers[launcher]
                .place_children(
                    (),
                    &self.context,
                    parent_size,
                    child_measurements,
                    placements,
                ),
            DesktopTarget::Project(project_id, ProjectPart::Matrix) => {
                self.place_project_matrix_children(*project_id, child_measurements, placements)
            }
            DesktopTarget::Instance(instance, part) => self.aggregates.instances[instance]
                .place_children(
                    *part,
                    &self.context,
                    parent_size,
                    child_measurements,
                    placements,
                ),
            _ => self.place_standard_children(id, parent_size, child_measurements, placements),
        }
    }

    fn measure(
        &self,
        id: &DesktopTarget,
        child_measurements: &[MeasuredLayout<2>],
    ) -> MeasuredLayout<2> {
        let child_sizes: Vec<_> = child_measurements.iter().map(|child| child.size).collect();

        match id {
            DesktopTarget::Launcher(launcher) => {
                self.aggregates.launchers[launcher].measure((), &self.context, child_measurements)
            }
            DesktopTarget::Project(project_id, ProjectPart::Header) => {
                self.project_header_size(*project_id)
            }
            DesktopTarget::Project(project_id, ProjectPart::Matrix) => self
                .measure_project_matrix(*project_id, &child_sizes)
                .into(),
            DesktopTarget::Instance(instance, part) => self.aggregates.instances[instance].measure(
                *part,
                &self.context,
                child_measurements,
            ),
            _ => self.measure_via_layout_spec(id, &child_sizes).into(),
        }
    }
}

impl DesktopLayoutAlgorithm<'_> {
    fn measure_via_layout_spec(&self, id: &DesktopTarget, child_sizes: &[Size<2>]) -> Size<2> {
        match self.resolve_layout_spec(id) {
            LayoutSpec::Leaf(size) => size.into(),
            LayoutSpec::Container {
                axis,
                padding,
                spacing,
            } => measure_container(axis, padding, spacing, child_sizes),
        }
    }

    fn measure_project_matrix(&self, project_id: ProjectId, child_sizes: &[Size<2>]) -> Size<2> {
        // Shared presented extents keep measured matrix bounds consistent with placement.
        matrix_size(&self.project_matrix_slots(project_id, child_sizes))
    }

    fn place_project_matrix_children(
        &self,
        project_id: ProjectId,
        child_measurements: &[MeasuredLayout<2>],
        placements: &mut [Placement<Transform, 2>],
    ) {
        let child_sizes: Vec<_> = child_measurements.iter().map(|child| child.size).collect();
        let slots = self.project_matrix_slots(project_id, &child_sizes);
        let (columns, rows) = matrix_tracks(&slots);

        for ((slot, measurement), placement) in slots.iter().zip(child_measurements).zip(placements)
        {
            let offset = Offset::from([
                track_offset(
                    &columns,
                    slot.placement.column as usize,
                    MATRIX_COLUMN_SPACING,
                ),
                track_offset(&rows, slot.placement.row as usize, MATRIX_ROW_SPACING),
            ]);
            let layout_size = expand_to_row(slot, measurement, &rows);
            let rect: RectPx = LayoutRect::new(offset, layout_size).into();
            let rect = Rect::from(rect);
            // The exact scaled center preserves left alignment despite rounded track extents.
            let center = rect.origin() + (rect.size() * slot.scale).center();
            let transform = Transform::new(center.with_z(0.0), Quaternion::IDENTITY, slot.scale);
            // Intrinsic rectangles keep presentation scaling from resizing the child scene.
            *placement = Placement::new(transform, LayoutRect::new(offset, layout_size));
        }
    }

    /// Cached intrinsic measurements keep parent scaling independent of child layout.
    fn project_matrix_slots(
        &self,
        project_id: ProjectId,
        child_sizes: &[Size<2>],
    ) -> Vec<MatrixSlot> {
        let contents = self.aggregates.hierarchy.matrix_slots(project_id);
        // ADR 0015: a parent-wide scale preserves the relative sizes of sibling projects.
        let widest_project = contents
            .iter()
            .zip(child_sizes)
            .filter_map(|(content, size)| {
                matches!(content, SlotContent::Project(_)).then_some(size[0])
            })
            .max()
            .unwrap_or(0);
        let project_scale =
            presentation_scale(self.context.default_panel_size.width, widest_project);

        contents
            .into_iter()
            .zip(child_sizes.iter().copied())
            .map(|(content, measured)| {
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
                    placement: self
                        .aggregates
                        .configuration
                        .placement_of_content(project_id, content)
                        .expect("slot content has a matrix placement"),
                }
            })
            .collect()
    }

    fn project_header_size(&self, project_id: ProjectId) -> MeasuredLayout<2> {
        let measured = self.aggregates.projects[&project_id].header.measured_size();
        let size: Size<2> = SizePx::new(
            measured.width,
            max(measured.height, PROJECT_HEADER_MIN_HEIGHT),
        )
        .into();
        MeasuredLayout::new(size, [true, false])
    }

    fn place_standard_children(
        &self,
        id: &DesktopTarget,
        parent_size: Size<2>,
        child_measurements: &[MeasuredLayout<2>],
        placements: &mut [Placement<Transform, 2>],
    ) {
        match self.resolve_layout_spec(id) {
            LayoutSpec::Leaf(_) => assert!(placements.is_empty(), "{id:?} is a leaf"),
            LayoutSpec::Container {
                axis,
                padding,
                spacing,
            } => {
                let cursor = Offset::from(padding.leading);
                let child_sizes =
                    expand_cross_axis_child_sizes(axis, padding, parent_size, child_measurements);
                place_container_children(axis, spacing as i32, cursor, &child_sizes, placements);
            }
        }
    }

    fn resolve_layout_spec(&self, target: &DesktopTarget) -> LayoutSpec {
        match target {
            // The desktop node lays out everything it contains vertically; the
            // root project is its only child.
            DesktopTarget::Desktop => LayoutAxis::VERTICAL.to_container().into(),
            DesktopTarget::Project(_, ProjectPart::Project) => LayoutAxis::VERTICAL
                .to_container()
                .spacing(PROJECT_HEADER_SPACING)
                .padding((PROJECT_PADDING, PROJECT_PADDING))
                .into(),
            // These targets are measured and placed by their own rules, never by a layout spec.
            DesktopTarget::Project(_, ProjectPart::Header | ProjectPart::Matrix)
            | DesktopTarget::Launcher(_)
            | DesktopTarget::Instance(..) => {
                panic!("{target:?} has no layout spec")
            }
        }
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

/// The size the tracks span, column and row spacing included.
fn matrix_size(slots: &[MatrixSlot]) -> Size<2> {
    let (columns, rows) = matrix_tracks(slots);
    [
        tracks_span(&columns, MATRIX_COLUMN_SPACING),
        tracks_span(&rows, MATRIX_ROW_SPACING),
    ]
    .into()
}

/// ADR 0015: width-only shrinking preserves readability without penalizing tall projects.
pub fn presentation_scale(preferred_width: u32, scene_width: u32) -> f64 {
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
fn matrix_tracks(slots: &[MatrixSlot]) -> (Vec<u32>, Vec<u32>) {
    let (column_count, row_count) = slots.iter().fold((0, 0), |(columns, rows), slot| {
        (
            max(columns, slot.placement.column as usize + 1),
            max(rows, slot.placement.row as usize + 1),
        )
    });
    let mut columns = vec![0; column_count];
    let mut rows = vec![0; row_count];
    for slot in slots {
        let column = slot.placement.column as usize;
        let row = slot.placement.row as usize;
        columns[column] = max(columns[column], slot.size[0]);
        rows[row] = max(rows[row], slot.size[1]);
    }
    (columns, rows)
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

/// The size of children stacked along `axis`: their extents and the spacing between them along it,
/// the largest extent across it.
pub fn measure_container(
    axis: LayoutAxis,
    padding: Thickness<2>,
    spacing: u32,
    child_sizes: &[Size<2>],
) -> Size<2> {
    let axis = *axis;
    let mut inner_size = Size::EMPTY;

    for (index, &child_size) in child_sizes.iter().enumerate() {
        for dim in 0..2 {
            if dim == axis {
                inner_size[dim] += child_size[dim];
                if index > 0 {
                    inner_size[dim] += spacing;
                }
            } else {
                inner_size[dim] = max(inner_size[dim], child_size[dim]);
            }
        }
    }

    padding.leading + inner_size + padding.trailing
}

pub fn place_container_children(
    axis: LayoutAxis,
    spacing: i32,
    mut cursor: Offset<2>,
    child_sizes: &[Size<2>],
    placements: &mut [Placement<Transform, 2>],
) {
    let axis_index: usize = axis.into();

    for (index, (&child_size, placement)) in child_sizes.iter().zip(placements).enumerate() {
        if index > 0 {
            cursor[axis_index] += spacing;
        }
        let rect: RectPx = LayoutRect::new(cursor, child_size).into();
        let center = rect.center().to_f64();
        let transform = Transform::from_translation(Vector3::new(center.x, center.y, 0.0));
        *placement = Placement::new(transform, LayoutRect::new(cursor, child_size));
        cursor[axis_index] += child_size[axis_index] as i32;
    }
}

fn expand_cross_axis_child_sizes(
    axis: LayoutAxis,
    padding: Thickness<2>,
    parent_size: Size<2>,
    child_measurements: &[MeasuredLayout<2>],
) -> Vec<Size<2>> {
    let axis_index: usize = axis.into();
    let cross_axis = 1 - axis_index;
    let cross_padding = padding.leading[cross_axis] + padding.trailing[cross_axis];
    let parent_content_cross_size = parent_size[cross_axis].saturating_sub(cross_padding);

    child_measurements
        .iter()
        .map(|child| {
            let mut child_size = child.size;
            if child.expandable_axes[cross_axis] {
                // Child sizes are minima from measure; placement may expand only on the cross axis
                // when the child opted in for this axis.
                child_size[cross_axis] = max(child_size[cross_axis], parent_content_cross_size);
            }
            child_size
        })
        .collect()
}

/// The extent of an instance for a regular `panel_size` (ADR 0019).
pub(crate) fn instance_extent(panel_size: SizePx, title_bar_height: u32) -> SizePx {
    SizePx::new(panel_size.width, panel_size.height + title_bar_height)
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
