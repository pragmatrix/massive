use std::cmp::max;

use derive_more::From;

use massive_geometry::{RectPx, SizePx, Transform, Vector3};
use massive_layout::{
    LayoutAlgorithm, LayoutAxis, MeasuredLayout, Offset, Placement, Rect as LayoutRect, Size,
    Thickness,
};

use massive_applications::InstanceId;

use super::{Aggregates, DesktopTarget};
use crate::layout::{ContainerBuilder, ToContainer};
use crate::projects::ProjectId;

const SECTION_SPACING: u32 = 20;
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
    pub default_panel_size: SizePx,
    pub focused_instance: Option<InstanceId>,
}

impl LayoutAlgorithm<DesktopTarget, Transform, 2> for DesktopLayoutAlgorithm<'_> {
    fn place_children(
        &self,
        id: &DesktopTarget,
        parent_size: Size<2>,
        child_measurements: &[MeasuredLayout<2>],
    ) -> Vec<Placement<Transform, 2>> {
        let child_sizes: Vec<_> = child_measurements.iter().map(|child| child.size).collect();

        match id {
            // Launcher panels run a dedicated path because transform assignment is
            // a second phase over the regular 2D child placement.
            DesktopTarget::Launcher(_) => self.place_launcher_children(id, &child_sizes),
            DesktopTarget::ProjectMatrix(project_id) => {
                self.place_project_matrix_children(*project_id, &child_sizes)
            }
            _ => self.place_standard_children(id, parent_size, child_measurements),
        }
    }

    fn measure(
        &self,
        id: &DesktopTarget,
        child_measurements: &[MeasuredLayout<2>],
    ) -> MeasuredLayout<2> {
        let child_sizes: Vec<_> = child_measurements.iter().map(|child| child.size).collect();

        match id {
            DesktopTarget::Launcher(launcher_id) => self.aggregates.launchers[launcher_id]
                .panel_measure_size(self.default_panel_size)
                .map(Into::into)
                .unwrap_or_else(|| self.measure_via_layout_spec(id, &child_sizes).into()),
            DesktopTarget::ProjectHeader(project_id) => self.project_header_size(*project_id),
            DesktopTarget::ProjectMatrix(project_id) => self
                .measure_project_matrix(*project_id, &child_sizes)
                .into(),
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
            } => {
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
        }
    }

    fn measure_project_matrix(&self, project_id: ProjectId, child_sizes: &[Size<2>]) -> Size<2> {
        let (columns, rows) = self.project_matrix_tracks(project_id, child_sizes);
        let matrix_width = tracks_span(&columns, MATRIX_COLUMN_SPACING);
        let matrix_height = tracks_span(&rows, MATRIX_ROW_SPACING);
        [matrix_width, matrix_height].into()
    }

    fn place_project_matrix_children(
        &self,
        project_id: ProjectId,
        child_sizes: &[Size<2>],
    ) -> Vec<Placement<Transform, 2>> {
        let (columns, rows) = self.project_matrix_tracks(project_id, child_sizes);
        let mut placements = Vec::with_capacity(child_sizes.len());

        for (launcher_id, child_size) in self
            .aggregates
            .hierarchy
            .matrix_launchers(project_id)
            .zip(child_sizes.iter().copied())
        {
            let placement = self.aggregates.matrix_positions[&launcher_id];
            let offset = Offset::from([
                track_offset(&columns, placement.column as usize, MATRIX_COLUMN_SPACING),
                track_offset(&rows, placement.row as usize, MATRIX_ROW_SPACING),
            ]);
            let rect: RectPx = LayoutRect::new(offset, child_size).into();
            let center = rect.center().to_f64();
            let transform = Transform::from_xy(center.x, center.y);
            placements.push(Placement::new(
                transform,
                LayoutRect::new(offset, child_size),
            ));
        }

        placements
    }

    fn project_matrix_tracks(
        &self,
        project_id: ProjectId,
        child_sizes: &[Size<2>],
    ) -> (Vec<u32>, Vec<u32>) {
        let mut columns = Vec::new();
        let mut rows = Vec::new();

        for (launcher_id, child_size) in self
            .aggregates
            .hierarchy
            .matrix_launchers(project_id)
            .zip(child_sizes.iter().copied())
        {
            let placement = self.aggregates.matrix_positions[&launcher_id];
            let column = placement.column as usize;
            let row = placement.row as usize;

            if columns.len() <= column {
                columns.resize(column + 1, 0);
            }
            if rows.len() <= row {
                rows.resize(row + 1, 0);
            }

            columns[column] = max(columns[column], child_size[0]);
            rows[row] = max(rows[row], child_size[1]);
        }

        (columns, rows)
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

    fn place_launcher_children(
        &self,
        id: &DesktopTarget,
        child_sizes: &[Size<2>],
    ) -> Vec<Placement<Transform, 2>> {
        let DesktopTarget::Launcher(launcher_id) = id else {
            panic!("place_launcher_children requires a launcher target")
        };

        let launcher = &self.aggregates.launchers[launcher_id];
        let child_instances = self.aggregates.hierarchy.launcher_instances(*launcher_id);

        // Performance: This don't need to be computed on non-visor launchers (but we might remove
        // bands anyway)
        let expanded = self
            .focused_instance
            .and_then(|focused| {
                child_instances
                    .iter()
                    .position(|&instance| instance == focused)
            })
            .is_some();

        launcher.place_panel_children(
            Offset::default(),
            child_sizes,
            &child_instances,
            expanded,
            self.default_panel_size,
        )
    }

    fn place_standard_children(
        &self,
        id: &DesktopTarget,
        parent_size: Size<2>,
        child_measurements: &[MeasuredLayout<2>],
    ) -> Vec<Placement<Transform, 2>> {
        match self.resolve_layout_spec(id) {
            LayoutSpec::Leaf(_) => Vec::new(),
            LayoutSpec::Container {
                axis,
                padding,
                spacing,
            } => {
                let cursor = Offset::from(padding.leading);
                let child_sizes =
                    expand_cross_axis_child_sizes(axis, padding, parent_size, child_measurements);
                place_container_children(axis, spacing as i32, cursor, &child_sizes)
            }
        }
    }

    fn resolve_layout_spec(&self, target: &DesktopTarget) -> LayoutSpec {
        match target {
            DesktopTarget::Desktop => LayoutAxis::VERTICAL
                .to_container()
                .spacing(SECTION_SPACING)
                .padding((0, 0))
                .into(),
            DesktopTarget::Project(_) => LayoutAxis::VERTICAL
                .to_container()
                .spacing(PROJECT_HEADER_SPACING)
                .padding((PROJECT_PADDING, PROJECT_PADDING))
                .into(),
            DesktopTarget::ProjectHeader(_) => {
                panic!("ProjectHeader is measured directly from header presenter")
            }
            DesktopTarget::ProjectMatrix(_) => {
                panic!("ProjectMatrix layout is handled by matrix placement")
            }
            DesktopTarget::Launcher(_) => {
                if self.aggregates.hierarchy.get_nested(target).is_empty() {
                    self.default_panel_size.into()
                } else {
                    LayoutAxis::HORIZONTAL.into()
                }
            }
            DesktopTarget::Instance(instance) => {
                let instance = &self.aggregates.instances[instance];
                if !instance.presents_primary_view() {
                    self.default_panel_size.into()
                } else {
                    LayoutAxis::HORIZONTAL.into()
                }
            }
            DesktopTarget::View(_) => self.default_panel_size.into(),
        }
    }
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

pub fn place_container_children(
    axis: LayoutAxis,
    spacing: i32,
    mut cursor: Offset<2>,
    child_sizes: &[Size<2>],
) -> Vec<Placement<Transform, 2>> {
    let axis_index: usize = axis.into();
    let mut child_placements = Vec::with_capacity(child_sizes.len());

    for (index, &child_size) in child_sizes.iter().enumerate() {
        if index > 0 {
            cursor[axis_index] += spacing;
        }
        let rect: RectPx = LayoutRect::new(cursor, child_size).into();
        let center = rect.center().to_f64();
        let transform = Transform::from_translation(Vector3::new(center.x, center.y, 0.0));
        child_placements.push(Placement::new(
            transform,
            LayoutRect::new(cursor, child_size),
        ));
        cursor[axis_index] += child_size[axis_index] as i32;
    }

    child_placements
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
