use std::cmp::max;

use massive_geometry::{RectPx, SizePx, Transform, Vector3};
use massive_layout::{
    LayoutAlgorithm, LayoutAxis, Offset, Rect as LayoutRect, Size, TransformOffset,
};

use massive_applications::InstanceId;

use super::{Aggregates, DesktopTarget};
use crate::layout::{LayoutSpec, ToContainer};

const SECTION_SPACING: u32 = 20;

pub(super) struct DesktopLayoutAlgorithm<'a> {
    pub(super) aggregates: &'a Aggregates,
    pub(super) default_panel_size: SizePx,
    pub(super) focused_instance: Option<InstanceId>,
}

impl LayoutAlgorithm<DesktopTarget, Transform, 2> for DesktopLayoutAlgorithm<'_> {
    fn place_children(
        &self,
        id: &DesktopTarget,
        parent_offset: Offset<2>,
        child_sizes: &[Size<2>],
    ) -> Vec<TransformOffset<Transform, 2>> {
        if let DesktopTarget::Launcher(_) = id {
            // Launcher panels run a dedicated path because transform assignment is
            // a second phase over the regular 2D child placement.
            return self.place_launcher_children(id, parent_offset, child_sizes);
        }

        self.place_standard_children(id, parent_offset, child_sizes)
    }

    fn measure(&self, id: &DesktopTarget, child_sizes: &[Size<2>]) -> Size<2> {
        if let DesktopTarget::Launcher(launcher_id) = id
            && let Some(size) =
                self.aggregates.launchers[launcher_id].panel_measure_size(self.default_panel_size)
        {
            return size;
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
    fn place_launcher_children(
        &self,
        id: &DesktopTarget,
        parent_offset: Offset<2>,
        child_sizes: &[Size<2>],
    ) -> Vec<TransformOffset<Transform, 2>> {
        let DesktopTarget::Launcher(launcher_id) = id else {
            panic!("place_launcher_children requires a launcher target")
        };

        let launcher = &self.aggregates.launchers[launcher_id];
        let children = self.aggregates.hierarchy.get_nested(id);
        let child_instances: Vec<_> = children
            .iter()
            .map(|target| match target {
                DesktopTarget::Instance(instance_id) => *instance_id,
                _ => panic!("launcher children must be instances"),
            })
            .collect();

        launcher.place_panel_children(
            parent_offset,
            child_sizes,
            &child_instances,
            self.default_panel_size,
            self.focused_instance,
        )
    }

    fn place_standard_children(
        &self,
        id: &DesktopTarget,
        parent_offset: Offset<2>,
        child_sizes: &[Size<2>],
    ) -> Vec<TransformOffset<Transform, 2>> {
        match self.resolve_layout_spec(id) {
            LayoutSpec::Leaf(_) => Vec::new(),
            LayoutSpec::Container {
                axis,
                padding,
                spacing,
            } => {
                let offset = parent_offset + Offset::from(padding.leading);
                place_container_children(axis, spacing as i32, offset, child_sizes)
            }
        }
    }

    fn resolve_layout_spec(&self, target: &DesktopTarget) -> LayoutSpec {
        match target {
            DesktopTarget::Desktop => LayoutAxis::VERTICAL
                .to_container()
                .spacing(SECTION_SPACING)
                .into(),
            DesktopTarget::Group(group_id) => self.aggregates.groups[group_id]
                .properties
                .layout
                .axis()
                .to_container()
                .spacing(10)
                .padding((10, 10))
                .into(),
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

pub(crate) fn place_container_children(
    axis: LayoutAxis,
    spacing: i32,
    mut offset: Offset<2>,
    child_sizes: &[Size<2>],
) -> Vec<TransformOffset<Transform, 2>> {
    let axis_index: usize = axis.into();
    let mut child_placements = Vec::with_capacity(child_sizes.len());

    for (index, &child_size) in child_sizes.iter().enumerate() {
        if index > 0 {
            offset[axis_index] += spacing;
        }
        let rect: RectPx = LayoutRect::new(offset, child_size).into();
        let center = rect.center().to_f64();
        let transform = Transform::from_translation(Vector3::new(center.x, center.y, 0.0));
        child_placements.push(TransformOffset::new(transform, offset));
        offset[axis_index] += child_size[axis_index] as i32;
    }

    child_placements
}
