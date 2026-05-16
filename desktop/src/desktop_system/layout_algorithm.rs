use std::cmp::max;

use derive_more::From;

use massive_geometry::{RectPx, SizePx, Transform, Vector3};
use massive_layout::{
    LayoutAlgorithm, LayoutAxis, Offset, Rect as LayoutRect, Size, Thickness, TransformOffset,
};

use massive_applications::InstanceId;

use super::{Aggregates, DesktopTarget};
use crate::layout::{ContainerBuilder, ToContainer};

const SECTION_SPACING: u32 = 20;
const PROJECT_PADDING: u32 = 10;
const PROJECT_HEADER_HEIGHT: u32 = 48;
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

pub(super) struct DesktopLayoutAlgorithm<'a> {
    pub(super) aggregates: &'a Aggregates,
    pub(super) default_panel_size: SizePx,
    pub(super) focused_instance: Option<InstanceId>,
}

impl LayoutAlgorithm<DesktopTarget, Transform, 2> for DesktopLayoutAlgorithm<'_> {
    fn place_children(
        &self,
        id: &DesktopTarget,
        child_sizes: &[Size<2>],
    ) -> Vec<TransformOffset<Transform, 2>> {
        if let DesktopTarget::Launcher(_) = id {
            // Launcher panels run a dedicated path because transform assignment is
            // a second phase over the regular 2D child placement.
            return self.place_launcher_children(id, child_sizes);
        }

        if let DesktopTarget::Project(_) = id {
            return self.place_project_children(id, child_sizes);
        }

        self.place_standard_children(id, child_sizes)
    }

    fn measure(&self, id: &DesktopTarget, child_sizes: &[Size<2>]) -> Size<2> {
        if let DesktopTarget::Launcher(launcher_id) = id
            && let Some(size) =
                self.aggregates.launchers[launcher_id].panel_measure_size(self.default_panel_size)
        {
            return size;
        }

        if let DesktopTarget::Project(_) = id {
            return self.measure_project(id, child_sizes);
        }

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
}

impl DesktopLayoutAlgorithm<'_> {
    fn measure_project(&self, id: &DesktopTarget, child_sizes: &[Size<2>]) -> Size<2> {
        let (columns, rows) = self.project_matrix_tracks(id, child_sizes);
        let matrix_width = tracks_span(&columns, MATRIX_COLUMN_SPACING);
        let matrix_height = tracks_span(&rows, MATRIX_ROW_SPACING);
        let width = PROJECT_PADDING * 2 + matrix_width;
        let height =
            PROJECT_PADDING * 2 + PROJECT_HEADER_HEIGHT + PROJECT_HEADER_SPACING + matrix_height;

        [width, height].into()
    }

    fn place_project_children(
        &self,
        id: &DesktopTarget,
        child_sizes: &[Size<2>],
    ) -> Vec<TransformOffset<Transform, 2>> {
        let children = self.aggregates.hierarchy.get_nested(id);
        let (columns, rows) = self.project_matrix_tracks(id, child_sizes);
        let mut placements = Vec::with_capacity(child_sizes.len());

        for (child, child_size) in children.iter().zip(child_sizes.iter().copied()) {
            let DesktopTarget::Launcher(launcher_id) = child else {
                panic!("Project children must be launchers")
            };
            let placement = self.aggregates.launcher_placements[launcher_id];
            let offset = Offset::from([
                PROJECT_PADDING as i32
                    + track_offset(&columns, placement.column as usize, MATRIX_COLUMN_SPACING),
                (PROJECT_PADDING + PROJECT_HEADER_HEIGHT + PROJECT_HEADER_SPACING) as i32
                    + track_offset(&rows, placement.row as usize, MATRIX_ROW_SPACING),
            ]);
            let rect: RectPx = LayoutRect::new(offset, child_size).into();
            let center = rect.center().to_f64();
            let transform = Transform::from_translation(Vector3::new(center.x, center.y, 0.0));
            placements.push(TransformOffset::new(transform, offset));
        }

        placements
    }

    fn project_matrix_tracks(
        &self,
        id: &DesktopTarget,
        child_sizes: &[Size<2>],
    ) -> (Vec<u32>, Vec<u32>) {
        let children = self.aggregates.hierarchy.get_nested(id);
        let mut columns = Vec::new();
        let mut rows = Vec::new();

        for (child, child_size) in children.iter().zip(child_sizes.iter().copied()) {
            let DesktopTarget::Launcher(launcher_id) = child else {
                panic!("Project children must be launchers")
            };
            let placement = self.aggregates.launcher_placements[launcher_id];
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

    fn place_launcher_children(
        &self,
        id: &DesktopTarget,
        child_sizes: &[Size<2>],
    ) -> Vec<TransformOffset<Transform, 2>> {
        let DesktopTarget::Launcher(launcher_id) = id else {
            panic!("place_launcher_children requires a launcher target")
        };

        let launcher = &self.aggregates.launchers[launcher_id];
        let child_instances = self.aggregates.launcher_instance_ids(*launcher_id);

        let focused_index = self.focused_instance.and_then(|focused| {
            child_instances
                .iter()
                .position(|&instance| instance == focused)
        });

        launcher.place_panel_children(
            Offset::default(),
            child_sizes,
            &child_instances,
            focused_index,
            self.default_panel_size,
        )
    }

    fn place_standard_children(
        &self,
        id: &DesktopTarget,
        child_sizes: &[Size<2>],
    ) -> Vec<TransformOffset<Transform, 2>> {
        match self.resolve_layout_spec(id) {
            LayoutSpec::Leaf(_) => Vec::new(),
            LayoutSpec::Container {
                axis,
                padding,
                spacing,
            } => {
                let cursor = Offset::from(padding.leading);
                place_container_children(axis, spacing as i32, cursor, child_sizes)
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
            DesktopTarget::Project(_) => panic!("Project layout is handled by matrix layout"),
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

pub(crate) fn place_container_children(
    axis: LayoutAxis,
    spacing: i32,
    mut cursor: Offset<2>,
    child_sizes: &[Size<2>],
) -> Vec<TransformOffset<Transform, 2>> {
    let axis_index: usize = axis.into();
    let mut child_placements = Vec::with_capacity(child_sizes.len());

    for (index, &child_size) in child_sizes.iter().enumerate() {
        if index > 0 {
            cursor[axis_index] += spacing;
        }
        let rect: RectPx = LayoutRect::new(cursor, child_size).into();
        let center = rect.center().to_f64();
        let transform = Transform::from_translation(Vector3::new(center.x, center.y, 0.0));
        child_placements.push(TransformOffset::new(transform, cursor));
        cursor[axis_index] += child_size[axis_index] as i32;
    }

    child_placements
}
