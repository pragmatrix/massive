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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::LaunchProfileId;
    use uuid::Uuid;

    fn instance_id() -> InstanceId {
        Uuid::new_v4().into()
    }

    fn view_id() -> massive_applications::ViewId {
        Uuid::new_v4().into()
    }

    fn launcher_id() -> LaunchProfileId {
        Uuid::new_v4().into()
    }

    fn hierarchy_with_instances(
        instances: &[InstanceId],
    ) -> (OrderedHierarchy<DesktopTarget>, LaunchProfileId) {
        let launcher = launcher_id();

        let mut hierarchy = OrderedHierarchy::default();
        hierarchy
            .add(DesktopTarget::Desktop, DesktopTarget::Launcher(launcher))
            .unwrap();

        for instance in instances {
            hierarchy
                .add(
                    DesktopTarget::Launcher(launcher),
                    DesktopTarget::Instance(*instance),
                )
                .unwrap();
        }

        (hierarchy, launcher)
    }

    #[test]
    fn resolve_neighbor_for_stopping_instance_returns_none_when_instance_is_not_focused() {
        let first = instance_id();
        let second = instance_id();
        let (hierarchy, _launcher) = hierarchy_with_instances(&[first, second]);

        let focused = DesktopTarget::Instance(second);
        let neighbor = hierarchy.resolve_neighbor_for_stopping_instance(Some(&focused), first);

        assert_eq!(neighbor, None);
    }

    #[test]
    fn resolve_neighbor_for_stopping_instance_returns_sibling_when_focused() {
        let first = instance_id();
        let second = instance_id();
        let (hierarchy, _launcher) = hierarchy_with_instances(&[first, second]);

        let focused = DesktopTarget::Instance(first);
        let neighbor = hierarchy.resolve_neighbor_for_stopping_instance(Some(&focused), first);

        assert_eq!(neighbor, Some(DesktopTarget::Instance(second)));
    }

    #[test]
    fn resolve_replacement_focus_for_stopping_instance_returns_none_when_target_not_in_focus_path()
    {
        let first = instance_id();
        let second = instance_id();
        let (hierarchy, _launcher) = hierarchy_with_instances(&[first, second]);

        let focused = DesktopTarget::Instance(second);
        let replacement =
            hierarchy.resolve_replacement_focus_for_stopping_instance(Some(&focused), first);

        assert_eq!(replacement, None);
    }

    #[test]
    fn resolve_replacement_focus_for_stopping_instance_prefers_neighbor_view() {
        let first = instance_id();
        let second = instance_id();
        let view = view_id();
        let (mut hierarchy, _launcher) = hierarchy_with_instances(&[first, second]);

        hierarchy
            .add(DesktopTarget::Instance(second), DesktopTarget::View(view))
            .unwrap();

        let focused = DesktopTarget::Instance(first);
        let replacement =
            hierarchy.resolve_replacement_focus_for_stopping_instance(Some(&focused), first);

        assert_eq!(replacement, Some(DesktopTarget::View(view)));
    }

    #[test]
    fn resolve_replacement_focus_for_stopping_instance_works_when_focus_is_view_inside_instance() {
        let first = instance_id();
        let second = instance_id();
        let first_view = view_id();
        let second_view = view_id();
        let (mut hierarchy, _launcher) = hierarchy_with_instances(&[first, second]);

        hierarchy
            .add(
                DesktopTarget::Instance(first),
                DesktopTarget::View(first_view),
            )
            .unwrap();
        hierarchy
            .add(
                DesktopTarget::Instance(second),
                DesktopTarget::View(second_view),
            )
            .unwrap();

        let focused = DesktopTarget::View(first_view);
        let replacement =
            hierarchy.resolve_replacement_focus_for_stopping_instance(Some(&focused), first);

        assert_eq!(replacement, Some(DesktopTarget::View(second_view)));
    }

    #[test]
    fn resolve_replacement_focus_for_stopping_instance_falls_back_to_parent() {
        let instance = instance_id();
        let (hierarchy, launcher) = hierarchy_with_instances(&[instance]);

        let focused = DesktopTarget::Instance(instance);
        let replacement =
            hierarchy.resolve_replacement_focus_for_stopping_instance(Some(&focused), instance);

        assert_eq!(replacement, Some(DesktopTarget::Launcher(launcher)));
    }

    #[test]
    fn resolve_neighbor_focus_target_prefers_single_view_of_instance() {
        let instance = instance_id();
        let view = view_id();
        let (mut hierarchy, _launcher) = hierarchy_with_instances(&[instance]);

        hierarchy
            .add(DesktopTarget::Instance(instance), DesktopTarget::View(view))
            .unwrap();

        let focus_target =
            hierarchy.resolve_neighbor_focus_target(&DesktopTarget::Instance(instance));
        assert_eq!(focus_target, DesktopTarget::View(view));
    }

    #[test]
    fn resolve_neighbor_focus_target_keeps_instance_without_view() {
        let instance = instance_id();
        let (hierarchy, _launcher) = hierarchy_with_instances(&[instance]);

        let focus_target =
            hierarchy.resolve_neighbor_focus_target(&DesktopTarget::Instance(instance));
        assert_eq!(focus_target, DesktopTarget::Instance(instance));
    }

    #[test]
    fn resolve_neighbor_focus_target_keeps_non_instance_target() {
        let launcher = launcher_id();
        let hierarchy = OrderedHierarchy::default();

        let focus_target =
            hierarchy.resolve_neighbor_focus_target(&DesktopTarget::Launcher(launcher));
        assert_eq!(focus_target, DesktopTarget::Launcher(launcher));
    }
}
