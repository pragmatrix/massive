use std::collections::HashMap;
use std::mem;

use massive_geometry::Transform;
use massive_layout::{
    LayoutAlgorithm, LayoutTopology, Offset, Placement, Rect as LayoutRect, Size as LayoutSize,
};

use super::DesktopTarget;
use crate::hit_tester::PlacementSource;

struct NativeLayoutBackend {
}

impl NativeLayoutBackend {
    fn place_subtree(
        target: &DesktopTarget,
        transform: Transform,
        offset: Offset<2>,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
        sizes: &HashMap<DesktopTarget, LayoutSize<2>>,
        placements: &mut Vec<(DesktopTarget, Placement<Transform, 2>)>,
    ) {
        let size = *sizes
            .get(target)
            .expect("Internal error: missing measured layout size for target");
        placements.push((
            target.clone(),
            Placement::new(transform, LayoutRect::new(offset, size)),
        ));

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

        let child_transforms = algorithm.place_children(target, offset, &child_sizes);
        if child_transforms.len() != children.len() {
            panic!("Internal error: child placement count does not match child count")
        }

        for (child, child_transform) in children.iter().zip(child_transforms.iter()) {
            Self::place_subtree(
                child,
                child_transform.transform,
                child_transform.offset,
                topology,
                algorithm,
                sizes,
                placements,
            );
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
    placements: HashMap<DesktopTarget, Placement<Transform, 2>>,
    staged_changed: Vec<(DesktopTarget, Placement<Transform, 2>)>,
}

impl DesktopLayoutState {
    pub(super) fn new() -> Self {
        Self {
            measured_sizes: HashMap::new(),
            placements: HashMap::new(),
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

    pub(super) fn place_from_target(
        &mut self,
        target: &DesktopTarget,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
    ) {
        if !topology.exists(target) {
            self.staged_changed.clear();
            return;
        }

        let (root_transform, root_offset) = if *target == DesktopTarget::Desktop {
            (Transform::default(), Offset::default())
        } else {
            let placement = self
                .placements
                .get(target)
                .unwrap_or_else(|| {
                    panic!(
                        "Internal error: targeted subtree placement requires existing absolute placement"
                    )
                });
            (placement.transform, placement.rect.offset)
        };

        let mut recomputed = Vec::new();
        NativeLayoutBackend::place_subtree(
            target,
            root_transform,
            root_offset,
            topology,
            algorithm,
            &self.measured_sizes,
            &mut recomputed,
        );

        self.staged_changed.clear();
        for (target, placement) in recomputed {
            let is_changed = self
                .placements
                .get(&target)
                .is_none_or(|current| current != &placement);
            self.placements.insert(target.clone(), placement);
            if is_changed {
                self.staged_changed.push((target, placement));
            }
        }

        self.placements.retain(|target, _| topology.exists(target));
    }

    pub(super) fn take_staged_changed(
        &mut self,
    ) -> Vec<(DesktopTarget, Placement<Transform, 2>)> {
        mem::take(&mut self.staged_changed)
    }

    pub(super) fn placement(&self, target: &DesktopTarget) -> Option<Placement<Transform, 2>> {
        self.placements.get(target).copied()
    }
}

impl PlacementSource for DesktopLayoutState {
    fn placement(&self, target: &DesktopTarget) -> Option<Placement<Transform, 2>> {
        DesktopLayoutState::placement(self, target)
    }
}
