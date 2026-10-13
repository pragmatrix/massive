use std::cmp::max;

use massive_geometry::{RectPx, Transform, Vector3};
use massive_layout::{
    ChildPlacement, LayoutAlgorithm, LayoutAxis, MeasuredLayout, Offset, Placement,
    Rect as LayoutRect, Size, Thickness,
};

use super::{Aggregates, DesktopTarget, LayoutContext, TargetLayout};
use crate::desktop_presenter::DesktopPresenter;

/// Routes each target to the presenter that lays it out, handing it the target's part (ADR 0022).
pub struct DesktopLayoutAlgorithm<'a> {
    pub desktop: &'a DesktopPresenter,
    pub aggregates: &'a Aggregates,
    pub context: LayoutContext<'a>,
}

impl LayoutAlgorithm<DesktopTarget, Transform, 2> for DesktopLayoutAlgorithm<'_> {
    fn place_children(&self, id: &DesktopTarget, children: ChildPlacement<'_, Transform, 2>) {
        let context = &self.context;
        match id {
            DesktopTarget::Desktop => self.desktop.place_children((), context, children),
            DesktopTarget::Project(project, part) => {
                self.aggregates.projects[project].place_children(*part, context, children)
            }
            DesktopTarget::Launcher(launcher) => {
                self.aggregates.launchers[launcher].place_children((), context, children)
            }
            DesktopTarget::Instance(instance, part) => {
                self.aggregates.instances[instance].place_children(*part, context, children)
            }
        }
    }

    fn measure(
        &self,
        id: &DesktopTarget,
        child_measurements: &[MeasuredLayout<2>],
    ) -> MeasuredLayout<2> {
        let context = &self.context;
        match id {
            DesktopTarget::Desktop => self.desktop.measure((), context, child_measurements),
            DesktopTarget::Project(project, part) => {
                self.aggregates.projects[project].measure(*part, context, child_measurements)
            }
            DesktopTarget::Launcher(launcher) => {
                self.aggregates.launchers[launcher].measure((), context, child_measurements)
            }
            DesktopTarget::Instance(instance, part) => {
                self.aggregates.instances[instance].measure(*part, context, child_measurements)
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
