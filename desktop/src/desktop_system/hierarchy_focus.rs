use massive_applications::InstanceId;

use super::DesktopTarget;
use crate::focus_path::PathResolver;
use crate::{DirectionBias, OrderedHierarchy};

impl OrderedHierarchy<DesktopTarget> {
    pub(super) fn resolve_replacement_focus_for_stopping_instance(
        &self,
        focused: Option<&DesktopTarget>,
        instance: InstanceId,
    ) -> Option<DesktopTarget> {
        let instance_target = DesktopTarget::Instance(instance);
        if !self.path_contains_target(focused, &instance_target) {
            return None;
        }

        if let Some(neighbor) = self.resolve_neighbor_for_stopping_instance(focused, instance) {
            return Some(self.resolve_neighbor_focus_target(&neighbor));
        }

        Some(
            self.parent(&instance_target)
                .expect("Internal error: instance has no parent")
                .clone(),
        )
    }

    pub(super) fn resolve_neighbor_for_stopping_instance(
        &self,
        focused: Option<&DesktopTarget>,
        instance: InstanceId,
    ) -> Option<DesktopTarget> {
        let focused_path = self.resolve_path(focused);
        if focused_path.instance() != Some(instance) {
            return None;
        }

        let instance_target = DesktopTarget::Instance(instance);
        self.entry(&instance_target)
            .neighbor(DirectionBias::Begin)
            .cloned()
    }

    pub(super) fn resolve_neighbor_focus_target(&self, neighbor: &DesktopTarget) -> DesktopTarget {
        match neighbor {
            DesktopTarget::Instance(_) => {
                if let [DesktopTarget::View(view)] = self.get_nested(neighbor) {
                    DesktopTarget::View(*view)
                } else {
                    neighbor.clone()
                }
            }
            _ => neighbor.clone(),
        }
    }

    pub(super) fn path_contains_target(
        &self,
        focused: Option<&DesktopTarget>,
        target: &DesktopTarget,
    ) -> bool {
        self.resolve_path(focused).contains(target)
    }
}
