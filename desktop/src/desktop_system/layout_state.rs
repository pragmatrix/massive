use std::collections::HashMap;

use massive_geometry::Transform;
use massive_layout::{LayoutAlgorithm, LayoutTopology, Offset, Placement, Rect as LayoutRect};

use super::DesktopTarget;
use crate::hit_tester::PlacementSource;

trait LayoutBackend {
    fn mark_reflow_pending(&mut self, target: DesktopTarget);
    fn recompute(
        &mut self,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
        absolute_offset: Offset<2>,
    ) -> Vec<(DesktopTarget, Placement<Transform, 2>)>;
}

struct NativeLayoutBackend {
    dirty: bool,
}

impl NativeLayoutBackend {
    fn new() -> Self {
        Self { dirty: true }
    }

    fn measure_subtree(
        target: &DesktopTarget,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
        sizes: &mut HashMap<DesktopTarget, massive_layout::Size<2>>,
    ) -> massive_layout::Size<2> {
        let child_sizes: Vec<_> = topology
            .children_of(target)
            .iter()
            .map(|child| Self::measure_subtree(child, topology, algorithm, sizes))
            .collect();

        let size = algorithm.measure(target, &child_sizes);
        sizes.insert(target.clone(), size);
        size
    }

    fn place_subtree(
        target: &DesktopTarget,
        transform: Transform,
        offset: Offset<2>,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
        sizes: &HashMap<DesktopTarget, massive_layout::Size<2>>,
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

impl LayoutBackend for NativeLayoutBackend {
    fn mark_reflow_pending(&mut self, _target: DesktopTarget) {
        self.dirty = true;
    }

    fn recompute(
        &mut self,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
        absolute_offset: Offset<2>,
    ) -> Vec<(DesktopTarget, Placement<Transform, 2>)> {
        if !self.dirty {
            return Vec::new();
        }

        self.dirty = false;

        let root = DesktopTarget::Desktop;
        if !topology.exists(&root) {
            return Vec::new();
        }

        let mut sizes = HashMap::new();
        Self::measure_subtree(&root, topology, algorithm, &mut sizes);

        let mut placements = Vec::new();
        Self::place_subtree(
            &root,
            Transform::default(),
            absolute_offset,
            topology,
            algorithm,
            &sizes,
            &mut placements,
        );

        placements
    }
}

pub(super) struct DesktopLayoutState {
    backend: NativeLayoutBackend,
    placements: HashMap<DesktopTarget, Placement<Transform, 2>>,
}

impl DesktopLayoutState {
    pub(super) fn new() -> Self {
        Self {
            backend: NativeLayoutBackend::new(),
            placements: HashMap::new(),
        }
    }

    pub(super) fn mark_reflow_pending(&mut self, target: DesktopTarget) {
        self.backend.mark_reflow_pending(target);
    }

    pub(super) fn recompute(
        &mut self,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
        absolute_offset: impl Into<Offset<2>>,
    ) -> Vec<(DesktopTarget, Placement<Transform, 2>)> {
        let recomputed = self
            .backend
            .recompute(topology, algorithm, absolute_offset.into());

        let mut changed = Vec::new();

        for (target, placement) in recomputed {
            let is_changed = self
                .placements
                .get(&target)
                .is_none_or(|current| current != &placement);

            self.placements.insert(target.clone(), placement);
            if is_changed {
                changed.push((target, placement));
            }
        }

        // Desktop-owned placement cache is the read source for hit testing and navigation.
        // Remove stale placements for nodes that no longer exist in the current topology.
        self.placements.retain(|target, _| topology.exists(target));

        changed
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
