use massive_geometry::Transform;
use massive_layout::{
    IncrementalLayouter, LayoutAlgorithm, LayoutTopology, Offset, Placement,
};

use super::DesktopTarget;
use crate::hit_tester::PlacementSource;

pub(super) struct DesktopLayoutState {
    layouter: IncrementalLayouter<DesktopTarget, Transform, 2>,
}

impl DesktopLayoutState {
    pub(super) fn new() -> Self {
        Self {
            layouter: IncrementalLayouter::with_initial_reflow(DesktopTarget::Desktop),
        }
    }

    pub(super) fn mark_reflow_pending(&mut self, target: DesktopTarget) {
        self.layouter.mark_reflow_pending(target);
    }

    pub(super) fn recompute(
        &mut self,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
        absolute_offset: impl Into<Offset<2>>,
    ) -> Vec<(DesktopTarget, Placement<Transform, 2>)> {
        self.layouter
            .recompute(topology, algorithm, absolute_offset)
            .changed
    }

    pub(super) fn placement(&self, target: &DesktopTarget) -> Option<Placement<Transform, 2>> {
        self.layouter.placement(target).copied()
    }
}

impl PlacementSource for DesktopLayoutState {
    fn placement(&self, target: &DesktopTarget) -> Option<Placement<Transform, 2>> {
        DesktopLayoutState::placement(self, target)
    }
}