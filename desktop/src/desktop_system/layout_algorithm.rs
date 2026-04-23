use std::cmp::max;

use massive_geometry::{RectPx, SizePx, Transform};
use massive_layout::{
    LayoutAlgorithm, LayoutAxis, Offset, Rect as LayoutRect, Size, TransformOffset,
};

use massive_applications::InstanceId;

use super::{Aggregates, DesktopTarget};
use crate::layout::{LayoutSpec, ToContainer};
use crate::projects::LauncherInstanceLayoutInput;

const SECTION_SPACING: u32 = 20;

pub(super) struct DesktopLayoutAlgorithm<'a> {
    pub(super) aggregates: &'a Aggregates,
    pub(super) default_panel_size: SizePx,
    pub(super) focused_instance: Option<InstanceId>,
}

impl DesktopLayoutAlgorithm<'_> {
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
        child_placements.push(TransformOffset::new(Transform::IDENTITY, offset));
        offset[axis_index] += child_size[axis_index] as i32;
    }

    child_placements
}

impl LayoutAlgorithm<DesktopTarget, Transform, 2> for DesktopLayoutAlgorithm<'_> {
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

    fn place_children(
        &self,
        id: &DesktopTarget,
        parent_offset: Offset<2>,
        child_sizes: &[Size<2>],
    ) -> Vec<TransformOffset<Transform, 2>> {
        if let DesktopTarget::Launcher(launcher_id) = id {
            let launcher = &self.aggregates.launchers[launcher_id];

            // Compute child offsets: custom (Visor) or default container layout (Band).
            let child_placements = if let Some(placements) =
                launcher.panel_child_offsets(parent_offset, child_sizes, self.default_panel_size)
            {
                placements
            } else {
                match self.resolve_layout_spec(id) {
                    LayoutSpec::Container {
                        axis,
                        padding,
                        spacing,
                    } => {
                        let offset = parent_offset + Offset::from(padding.leading);
                        place_container_children(axis, spacing as i32, offset, child_sizes)
                    }
                    _ => Vec::new(),
                }
            };

            // Compute 3D transforms for instance children.
            let children = self.aggregates.hierarchy.get_nested(id);
            let instance_inputs: Vec<LauncherInstanceLayoutInput> = children
                .iter()
                .zip(child_placements.iter().zip(child_sizes.iter()))
                .filter_map(|(target, (child_transform_offset, size))| match target {
                    DesktopTarget::Instance(instance_id) => {
                        let rect_px: RectPx =
                            LayoutRect::new(child_transform_offset.offset, *size).into();
                        Some(LauncherInstanceLayoutInput {
                            instance_id: *instance_id,
                            rect: rect_px,
                        })
                    }
                    _ => None,
                })
                .collect();

            let layout_targets =
                launcher.compute_instance_layout_targets(&instance_inputs, self.focused_instance);

            // Rebuild placements: match each child with its computed transform.
            let mut transform_by_instance: std::collections::HashMap<InstanceId, Transform> =
                layout_targets
                    .into_iter()
                    .map(|lt| (lt.instance_id, lt.layout_transform))
                    .collect();

            return children
                .iter()
                .zip(child_placements)
                .map(|(target, child_transform_offset)| {
                    let transform = match target {
                        DesktopTarget::Instance(instance_id) => transform_by_instance
                            .remove(instance_id)
                            .unwrap_or(child_transform_offset.transform),
                        _ => child_transform_offset.transform,
                    };
                    TransformOffset::new(transform, child_transform_offset.offset)
                })
                .collect();
        }

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
}
