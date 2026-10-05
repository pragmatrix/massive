use std::cmp::max;

use derive_more::From;

use massive_applications::InstanceId;
use massive_geometry::{Point, Quaternion, Rect, RectPx, SizePx, Transform, Vector3};
use massive_layout::{
    LayoutAlgorithm, LayoutAxis, MeasuredLayout, Offset, Placement, Rect as LayoutRect, Size,
    Thickness,
};

use super::{Aggregates, DesktopTarget, fullscreen_scale};
use crate::layout::{ContainerBuilder, ToContainer};
use crate::projects::{
    FullScreenMode, LaunchProfileId, LauncherMode, MatrixPlacement, ProjectId, SlotIds,
    launcher_mode,
};

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
    pub window_size: SizePx,
}

impl DesktopLayoutAlgorithm<'_> {
    /// Whether `instance` presents in Full Screen Mode (ADR 0014): the content
    /// scale follows the instance's mode — a base instance's launcher mode, an
    /// assistant's temporary one — independent of camera or focus depth.
    fn is_instance_full_screen(&self, instance: InstanceId) -> bool {
        self.aggregates.instance_full_screen_mode(instance) == FullScreenMode::FullScreen
    }

    /// The mode of a launcher, resolved through the topology's parent links and
    /// the hosting matrix.
    fn launcher_mode(&self, launcher_id: LaunchProfileId) -> LauncherMode {
        let project = self.aggregates.hierarchy.project_of_launcher(launcher_id);
        self.aggregates.configuration[project]
            .launcher(launcher_id)
            .expect("the hosting matrix holds the launcher")
            .mode
    }
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
            DesktopTarget::Instance(instance_id) => {
                self.place_instance_children(*instance_id, parent_size, child_measurements)
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
            DesktopTarget::Launcher(launcher_id) => {
                let panel_measure: Option<Size<2>> = launcher_mode::panel_measurement(
                    self.launcher_mode(*launcher_id),
                    self.default_panel_size,
                );
                panel_measure
                    .map(Into::into)
                    .unwrap_or_else(|| self.measure_via_layout_spec(id, &child_sizes).into())
            }
            DesktopTarget::ProjectHeader(project_id) => self.project_header_size(*project_id),
            DesktopTarget::ProjectMatrix(project_id) => self
                .measure_project_matrix(*project_id, &child_sizes)
                .into(),
            DesktopTarget::Instance(_) => {
                let size: Size<2> = self.default_panel_size.into();
                size.into()
            }
            DesktopTarget::View(_) => {
                let is_fullscreen = self
                    .aggregates
                    .hierarchy
                    .instance_of_target(id)
                    .is_some_and(|inst| self.is_instance_full_screen(inst));
                let size_px = if is_fullscreen {
                    self.window_size
                } else {
                    self.default_panel_size
                };
                let size: Size<2> = size_px.into();
                size.into()
            }
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
        // Shared presented extents keep measured matrix bounds consistent with placement.
        matrix_size(&self.project_matrix_slots(project_id, child_sizes))
    }

    fn place_project_matrix_children(
        &self,
        project_id: ProjectId,
        child_sizes: &[Size<2>],
    ) -> Vec<Placement<Transform, 2>> {
        let slots = self.project_matrix_slots(project_id, child_sizes);
        let (columns, rows) = matrix_tracks(&slots);
        let mut placements = Vec::with_capacity(slots.len());

        for slot in &slots {
            let offset = Offset::from([
                track_offset(
                    &columns,
                    slot.placement.column as usize,
                    MATRIX_COLUMN_SPACING,
                ),
                track_offset(&rows, slot.placement.row as usize, MATRIX_ROW_SPACING),
            ]);
            let rect: RectPx = LayoutRect::new(offset, slot.layout_size).into();
            let rect = Rect::from(rect);
            // The exact scaled center preserves left alignment despite rounded track extents.
            let center = rect.origin() + (rect.size() * slot.scale).center();
            let transform = Transform::new(center.with_z(0.0), Quaternion::IDENTITY, slot.scale);
            // Intrinsic rectangles keep presentation scaling from resizing the child scene.
            placements.push(Placement::new(
                transform,
                LayoutRect::new(offset, slot.layout_size),
            ));
        }

        placements
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
            .filter_map(|(content, size)| matches!(content, SlotIds::Project(_)).then_some(size[0]))
            .max()
            .unwrap_or(0);
        let project_scale = presentation_scale(self.default_panel_size.width, widest_project);

        contents
            .into_iter()
            .zip(child_sizes.iter().copied())
            .map(|(content, measured)| {
                let scale = match content {
                    SlotIds::Launcher(_) => 1.0,
                    SlotIds::Project(_) => project_scale,
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
                        .project(project_id)
                        .and_then(|project| project.placement_of_content(content))
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

    fn place_launcher_children(
        &self,
        id: &DesktopTarget,
        child_sizes: &[Size<2>],
    ) -> Vec<Placement<Transform, 2>> {
        let DesktopTarget::Launcher(launcher_id) = id else {
            panic!("place_launcher_children requires a launcher target")
        };

        let launcher = &self.aggregates.launchers[launcher_id];
        let child_instances: Vec<_> = self
            .aggregates
            .hierarchy
            .launcher_instances(*launcher_id)
            .collect();

        // Performance: This don't need to be computed on non-visor launchers (but we might remove
        // bands anyway). The visor only collapses when one of its instances holds keyboard focus.
        let expanded = self
            .focused_instance
            .and_then(|focused| {
                child_instances
                    .iter()
                    .position(|&instance| instance == focused)
            })
            .is_some();

        // The pack decision is a mode policy; the visor branch stays on the
        // presenter because it places children around the presenter's focus anchor.
        launcher_mode::place_panel_children(
            self.launcher_mode(*launcher_id),
            launcher,
            Offset::default(),
            child_sizes,
            &child_instances,
            expanded,
            self.default_panel_size,
        )
    }

    fn place_instance_children(
        &self,
        instance_id: InstanceId,
        parent_size: Size<2>,
        child_measurements: &[MeasuredLayout<2>],
    ) -> Vec<Placement<Transform, 2>> {
        let is_fullscreen = self.is_instance_full_screen(instance_id);
        let center = Point::new(parent_size[0] as f64 * 0.5, parent_size[1] as f64 * 0.5);

        child_measurements
            .iter()
            .map(|child| {
                let view_size = child.size;
                let (scale, placement_size) = if is_fullscreen {
                    (
                        fullscreen_scale(
                            SizePx::new(parent_size[0], parent_size[1]),
                            self.window_size,
                        ),
                        [self.window_size.width, self.window_size.height].into(),
                    )
                } else {
                    (1.0, view_size)
                };

                let transform = Transform::new(
                    Vector3::new(center.x, center.y, 0.0),
                    Quaternion::IDENTITY,
                    scale,
                );
                Placement::new(
                    transform,
                    LayoutRect::new(Offset::default(), placement_size),
                )
            })
            .collect()
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
            // The desktop node lays out everything it contains vertically; the
            // root project is its only child.
            DesktopTarget::Desktop => LayoutAxis::VERTICAL.to_container().into(),
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
            DesktopTarget::Instance(_) => self.default_panel_size.into(),
            DesktopTarget::View(_) => self.default_panel_size.into(),
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

/// The column and row tracks of a matrix: each track the largest slot in it.
fn matrix_tracks(slots: &[MatrixSlot]) -> (Vec<u32>, Vec<u32>) {
    let mut columns = Vec::new();
    let mut rows = Vec::new();

    for slot in slots {
        let column = slot.placement.column as usize;
        let row = slot.placement.row as usize;

        if columns.len() <= column {
            columns.resize(column + 1, 0);
        }
        if rows.len() <= row {
            rows.resize(row + 1, 0);
        }

        columns[column] = max(columns[column], slot.size[0]);
        rows[row] = max(rows[row], slot.size[1]);
    }

    (columns, rows)
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
}
