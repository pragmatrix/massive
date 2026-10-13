use std::cmp::max;

use derive_more::From;

use massive_geometry::{RectPx, SizePx, Transform, Vector3};
use massive_layout::{
    LayoutAlgorithm, LayoutAxis, MeasuredLayout, Offset, Placement, Rect as LayoutRect, Size,
    Thickness,
};

use super::{Aggregates, DesktopTarget, LayoutContext, TargetLayout};
use crate::layout::{ContainerBuilder, ToContainer};

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
            DesktopTarget::Project(project, part) => self.aggregates.projects[project]
                .place_children(
                    *part,
                    &self.context,
                    parent_size,
                    child_measurements,
                    placements,
                ),
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
        match id {
            DesktopTarget::Launcher(launcher) => {
                self.aggregates.launchers[launcher].measure((), &self.context, child_measurements)
            }
            DesktopTarget::Project(project, part) => {
                self.aggregates.projects[project].measure(*part, &self.context, child_measurements)
            }
            DesktopTarget::Instance(instance, part) => self.aggregates.instances[instance].measure(
                *part,
                &self.context,
                child_measurements,
            ),
            _ => self.measure_via_layout_spec(id, child_measurements).into(),
        }
    }
}

impl DesktopLayoutAlgorithm<'_> {
    fn measure_via_layout_spec(
        &self,
        id: &DesktopTarget,
        child_measurements: &[MeasuredLayout<2>],
    ) -> Size<2> {
        match self.resolve_layout_spec(id) {
            LayoutSpec::Leaf(size) => size.into(),
            LayoutSpec::Container {
                axis,
                padding,
                spacing,
            } => measure_container(
                axis,
                padding,
                spacing,
                child_measurements.iter().map(|child| child.size),
            ),
        }
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
                place_container_children(axis, spacing as i32, cursor, child_sizes, placements);
            }
        }
    }

    fn resolve_layout_spec(&self, target: &DesktopTarget) -> LayoutSpec {
        match target {
            // The desktop node lays out everything it contains vertically; the
            // root project is its only child.
            DesktopTarget::Desktop => LayoutAxis::VERTICAL.to_container().into(),
            // These targets are measured and placed by their own rules, never by a layout spec.
            DesktopTarget::Project(..)
            | DesktopTarget::Launcher(_)
            | DesktopTarget::Instance(..) => {
                panic!("{target:?} has no layout spec")
            }
        }
    }
}

/// The size of children stacked along `axis`: their extents and the spacing between them along it,
/// the largest extent across it.
pub fn measure_container(
    axis: LayoutAxis,
    padding: Thickness<2>,
    spacing: u32,
    child_sizes: impl IntoIterator<Item = Size<2>>,
) -> Size<2> {
    let axis = *axis;
    let mut inner_size = Size::EMPTY;

    for (index, child_size) in child_sizes.into_iter().enumerate() {
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
    child_sizes: impl IntoIterator<Item = Size<2>>,
    placements: &mut [Placement<Transform, 2>],
) {
    let axis_index: usize = axis.into();

    for (index, (child_size, placement)) in child_sizes.into_iter().zip(placements).enumerate() {
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

pub fn expand_cross_axis_child_sizes(
    axis: LayoutAxis,
    padding: Thickness<2>,
    parent_size: Size<2>,
    child_measurements: &[MeasuredLayout<2>],
) -> impl Iterator<Item = Size<2>> + '_ {
    let axis_index: usize = axis.into();
    let cross_axis = 1 - axis_index;
    let cross_padding = padding.leading[cross_axis] + padding.trailing[cross_axis];
    let parent_content_cross_size = parent_size[cross_axis].saturating_sub(cross_padding);

    child_measurements.iter().map(move |child| {
        let mut child_size = child.size;
        if child.expandable_axes[cross_axis] {
            // Child sizes are minima from measure; placement may expand only on the cross axis
            // when the child opted in for this axis.
            child_size[cross_axis] = max(child_size[cross_axis], parent_content_cross_size);
        }
        child_size
    })
}

/// The extent of an instance for a regular `panel_size` (ADR 0019).
pub(crate) fn instance_extent(panel_size: SizePx, title_bar_height: u32) -> SizePx {
    SizePx::new(panel_size.width, panel_size.height + title_bar_height)
}
