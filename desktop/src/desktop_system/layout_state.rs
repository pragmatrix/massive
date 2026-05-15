use std::collections::HashMap;
use std::mem;

use massive_geometry::{Point, Transform, Vector3};
use massive_layout::{
    LayoutAlgorithm, LayoutTopology, Offset, Placement, Rect as LayoutRect, Size as LayoutSize,
};

use super::DesktopTarget;
use crate::OrderedHierarchy;
use crate::hit_tester::PlacementSource;

struct NativeLayoutBackend {}

impl NativeLayoutBackend {
    fn place_children(
        target: &DesktopTarget,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
        sizes: &HashMap<DesktopTarget, LayoutSize<2>>,
        placements: &mut Vec<(DesktopTarget, Placement<Transform, 2>)>,
    ) {
        let children = topology.children_of(target);
        if children.is_empty() {
            return;
        }

        let child_sizes: Vec<_> = children
            .iter()
            .map(|child| {
                *sizes
                    .get(child)
                    .expect("Internal error: missing measured layout size for child")
            })
            .collect();

        let child_transforms = algorithm.place_children(target, &child_sizes);
        if child_transforms.len() != children.len() {
            panic!("Internal error: child placement count does not match child count")
        }

        for (child, child_transform) in children.iter().zip(child_transforms.iter()) {
            let size = *sizes
                .get(child)
                .expect("Internal error: missing measured layout size for child");
            placements.push((
                child.clone(),
                Placement::new(
                    child_transform.transform,
                    LayoutRect::new(child_transform.offset, size),
                ),
            ));
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct MeasureOutcome {
    pub(super) size_changed: bool,
    pub(super) parent: Option<DesktopTarget>,
}

pub(super) struct DesktopLayoutState {
    measured_sizes: HashMap<DesktopTarget, LayoutSize<2>>,
    local_placements: HashMap<DesktopTarget, Placement<Transform, 2>>,
    staged_changed: Vec<(DesktopTarget, Placement<Transform, 2>)>,
}

impl DesktopLayoutState {
    pub(super) fn new() -> Self {
        Self {
            measured_sizes: HashMap::new(),
            local_placements: HashMap::new(),
            staged_changed: Vec::new(),
        }
    }

    pub(super) fn missing_child_measures(
        &self,
        target: &DesktopTarget,
        topology: &impl LayoutTopology<DesktopTarget>,
    ) -> Vec<DesktopTarget> {
        topology
            .children_of(target)
            .iter()
            .filter(|child| !self.measured_sizes.contains_key(*child))
            .cloned()
            .collect()
    }

    pub(super) fn measure_node(
        &mut self,
        target: &DesktopTarget,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
    ) -> MeasureOutcome {
        let child_sizes: Vec<_> = topology
            .children_of(target)
            .iter()
            .map(|child| {
                *self.measured_sizes.get(child).unwrap_or_else(|| {
                    panic!("Internal error: child should be measured before parent")
                })
            })
            .collect();

        let measured = algorithm.measure(target, &child_sizes);
        let size_changed = self
            .measured_sizes
            .get(target)
            .is_none_or(|current| current != &measured);
        self.measured_sizes.insert(target.clone(), measured);

        MeasureOutcome {
            size_changed,
            parent: topology.parent_of(target),
        }
    }

    pub(super) fn place_children_of(
        &mut self,
        target: &DesktopTarget,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
    ) {
        if !topology.exists(target) {
            return;
        }

        self.update_root_placement(target);

        let mut recomputed = Vec::new();
        NativeLayoutBackend::place_children(
            target,
            topology,
            algorithm,
            &self.measured_sizes,
            &mut recomputed,
        );

        for (target, placement) in recomputed {
            let is_changed = self
                .local_placements
                .get(&target)
                .is_none_or(|current| current != &placement);
            self.local_placements.insert(target.clone(), placement);
            if is_changed {
                self.stage_changed(target, placement);
            }
        }

        self.local_placements
            .retain(|target, _| topology.exists(target));
        self.measured_sizes
            .retain(|target, _| topology.exists(target));
    }

    pub(super) fn take_staged_changed(&mut self) -> Vec<(DesktopTarget, Placement<Transform, 2>)> {
        mem::take(&mut self.staged_changed)
    }

    fn stage_changed(&mut self, target: DesktopTarget, placement: Placement<Transform, 2>) {
        self.staged_changed
            .retain(|(staged_target, _)| staged_target != &target);
        self.staged_changed.push((target, placement));
    }

    pub(super) fn absolute_placement(
        &self,
        target: &DesktopTarget,
        topology: &impl LayoutTopology<DesktopTarget>,
    ) -> Option<Placement<Transform, 2>> {
        let mut path = Vec::new();
        let mut current = target.clone();
        loop {
            path.push(current.clone());
            let Some(parent) = topology.parent_of(&current) else {
                break;
            };
            current = parent;
        }

        let mut origin_transform = Transform::default();
        let mut offset = Offset::default();
        for path_target in path.iter().rev() {
            let placement = self.local_placements.get(path_target)?;
            let local_origin_transform = if *path_target == DesktopTarget::Desktop {
                // Desktop transform is already origin-based (IDENTITY in the common case).
                placement.transform
            } else {
                let local_center = Self::layout_local_center(placement.rect.size);
                Self::transform_with_layout(placement.transform, local_center)
            };
            origin_transform *= local_origin_transform;
            offset += placement.rect.offset;
        }

        let local = self.local_placements.get(target)?;
        let local_center = Self::layout_local_center(local.rect.size);
        let local_center = Vector3::new(local_center.x, local_center.y, 0.0);
        let center_translation = origin_transform.translate
            + origin_transform.rotate * (local_center * origin_transform.scale);

        let transform = Transform::new(
            center_translation,
            origin_transform.rotate,
            origin_transform.scale,
        );

        Some(Placement::new(
            transform,
            LayoutRect::new(offset, local.rect.size),
        ))
    }

    fn layout_local_center(size: LayoutSize<2>) -> Point {
        Point::new(size[0] as f64 * 0.5, size[1] as f64 * 0.5)
    }

    fn transform_with_layout(layout_transform: Transform, local_center: Point) -> Transform {
        let local_center = Vector3::new(local_center.x, local_center.y, 0.0);
        let origin_translation =
            layout_transform.translate + layout_transform.rotate * -local_center;
        Transform::new(
            origin_translation,
            layout_transform.rotate,
            layout_transform.scale,
        )
    }

    fn update_root_placement(&mut self, target: &DesktopTarget) {
        if *target != DesktopTarget::Desktop {
            return;
        }

        let size = *self
            .measured_sizes
            .get(target)
            .expect("Internal error: missing measured layout size for desktop root");
        self.local_placements.insert(
            target.clone(),
            Placement::new(
                Transform::default(),
                LayoutRect::new(Offset::default(), size),
            ),
        );
    }
}

impl PlacementSource for DesktopLayoutState {
    fn placement(
        &self,
        target: &DesktopTarget,
        hierarchy: &OrderedHierarchy<DesktopTarget>,
    ) -> Option<Placement<Transform, 2>> {
        self.absolute_placement(target, hierarchy)
    }
}
