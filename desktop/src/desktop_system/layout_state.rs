use std::collections::HashMap;

use massive_geometry::Transform;
use massive_layout::{IncrementalLayouter, LayoutAlgorithm, LayoutTopology, Offset, Placement};

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

struct IncrementalLayoutBackend {
    layouter: IncrementalLayouter<DesktopTarget, Transform, 2>,
}

impl IncrementalLayoutBackend {
    fn new() -> Self {
        Self {
            layouter: IncrementalLayouter::with_initial_reflow(DesktopTarget::Desktop),
        }
    }
}

impl LayoutBackend for IncrementalLayoutBackend {
    fn mark_reflow_pending(&mut self, target: DesktopTarget) {
        self.layouter.mark_reflow_pending(target);
    }

    fn recompute(
        &mut self,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
        absolute_offset: Offset<2>,
    ) -> Vec<(DesktopTarget, Placement<Transform, 2>)> {
        self.layouter
            .recompute(topology, algorithm, absolute_offset)
            .changed
    }

}

pub(super) struct DesktopLayoutState {
    backend: IncrementalLayoutBackend,
    placements: HashMap<DesktopTarget, Placement<Transform, 2>>,
}

impl DesktopLayoutState {
    pub(super) fn new() -> Self {
        Self {
            backend: IncrementalLayoutBackend::new(),
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
        let changed = self
            .backend
            .recompute(topology, algorithm, absolute_offset.into())
            ;

        // Desktop-owned placement cache is the read source for hit testing and navigation.
        for (target, placement) in &changed {
            self.placements.insert(target.clone(), *placement);
        }

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
