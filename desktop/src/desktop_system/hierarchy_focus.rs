use massive_applications::InstanceId;

use super::{DesktopTarget, FocusableTarget, InstancePart};
use crate::focus_path::PathResolver;
use crate::policy::when::{self, InstanceSuccessor};
use crate::{DirectionBias, OrderedHierarchy};

impl OrderedHierarchy<DesktopTarget> {
    pub(super) fn resolve_replacement_focus_for_stopping_instance(
        &self,
        focused: &DesktopTarget,
        instance: InstanceId,
    ) -> Option<FocusableTarget> {
        if !self.path_contains_target(Some(focused), &instance.into()) {
            return None;
        }

        when::stopping_focused_instance()
            .into_iter()
            .find_map(|successor| match successor {
                InstanceSuccessor::Neighbor(bias) => self
                    .resolve_neighbor_for_stopping_instance(focused, instance, bias)
                    .map(|neighbor| self.instance_focus_target(neighbor)),
                InstanceSuccessor::Launcher => Some(FocusableTarget::Launcher(
                    self.launcher_of_instance(instance),
                )),
            })
    }

    fn resolve_neighbor_for_stopping_instance(
        &self,
        focused: &DesktopTarget,
        instance: InstanceId,
        bias: DirectionBias,
    ) -> Option<InstanceId> {
        if self.resolve_path(Some(focused)).instance() != Some(instance) {
            return None;
        }
        let neighbor = self.entry(&instance.into()).neighbor(bias).cloned()?;
        let DesktopTarget::Instance(neighbor, InstancePart::Instance) = neighbor else {
            panic!("A launcher nests only instances, found {neighbor:?}");
        };
        Some(neighbor)
    }

    /// Resolves an instance to its sole view when present, so keyboard focus reaches the view;
    /// otherwise, the instance itself.
    pub(super) fn instance_focus_target(&self, instance: InstanceId) -> FocusableTarget {
        self.get_nested(&instance.into())
            .iter()
            .find_map(|nested| match nested {
                DesktopTarget::Instance(_, InstancePart::View(view)) => {
                    Some(FocusableTarget::View(instance, *view))
                }
                _ => None,
            })
            .unwrap_or(FocusableTarget::Instance(instance))
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
    use crate::projects::{LaunchProfileId, ProjectId};
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
        let project = ProjectId::new();

        let mut hierarchy = OrderedHierarchy::default();
        hierarchy
            .add_nested(
                project.into(),
                [
                    DesktopTarget::header(project),
                    DesktopTarget::matrix(project),
                ],
            )
            .unwrap();
        hierarchy
            .add(
                DesktopTarget::matrix(project),
                DesktopTarget::Launcher(launcher),
            )
            .unwrap();

        for instance in instances {
            hierarchy
                .add(
                    DesktopTarget::Launcher(launcher),
                    DesktopTarget::from(*instance),
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

        let focused = second.into();
        let neighbor =
            hierarchy.resolve_neighbor_for_stopping_instance(&focused, first, DirectionBias::Begin);

        assert_eq!(neighbor, None);
    }

    #[test]
    fn resolve_neighbor_for_stopping_instance_returns_sibling_when_focused() {
        let first = instance_id();
        let second = instance_id();
        let (hierarchy, _launcher) = hierarchy_with_instances(&[first, second]);

        let focused = first.into();
        let neighbor =
            hierarchy.resolve_neighbor_for_stopping_instance(&focused, first, DirectionBias::Begin);

        assert_eq!(neighbor, Some(second));
    }

    #[test]
    fn resolve_replacement_focus_for_stopping_instance_returns_none_when_target_not_in_focus_path()
    {
        let first = instance_id();
        let second = instance_id();
        let (hierarchy, _launcher) = hierarchy_with_instances(&[first, second]);

        let focused = second.into();
        let replacement =
            hierarchy.resolve_replacement_focus_for_stopping_instance(&focused, first);

        assert_eq!(replacement, None);
    }

    #[test]
    fn resolve_replacement_focus_for_stopping_instance_prefers_neighbor_view() {
        let first = instance_id();
        let second = instance_id();
        let view = view_id();
        let (mut hierarchy, _launcher) = hierarchy_with_instances(&[first, second]);

        hierarchy
            .add(second.into(), DesktopTarget::view(second, view))
            .unwrap();

        let focused = first.into();
        let replacement =
            hierarchy.resolve_replacement_focus_for_stopping_instance(&focused, first);

        assert_eq!(replacement, Some(FocusableTarget::View(second, view)));
    }

    #[test]
    fn resolve_replacement_focus_for_stopping_instance_works_when_focus_is_view_inside_instance() {
        let first = instance_id();
        let second = instance_id();
        let first_view = view_id();
        let second_view = view_id();
        let (mut hierarchy, _launcher) = hierarchy_with_instances(&[first, second]);

        hierarchy
            .add(first.into(), DesktopTarget::view(first, first_view))
            .unwrap();
        hierarchy
            .add(second.into(), DesktopTarget::view(second, second_view))
            .unwrap();

        let focused = DesktopTarget::view(first, first_view);
        let replacement =
            hierarchy.resolve_replacement_focus_for_stopping_instance(&focused, first);

        assert_eq!(
            replacement,
            Some(FocusableTarget::View(second, second_view))
        );
    }

    #[test]
    fn resolve_replacement_focus_for_stopping_instance_falls_back_to_parent() {
        let instance = instance_id();
        let (hierarchy, launcher) = hierarchy_with_instances(&[instance]);

        let focused = instance.into();
        let replacement =
            hierarchy.resolve_replacement_focus_for_stopping_instance(&focused, instance);

        assert_eq!(replacement, Some(FocusableTarget::Launcher(launcher)));
    }

    #[test]
    fn instance_focus_target_prefers_single_view_of_instance() {
        let instance = instance_id();
        let view = view_id();
        let (mut hierarchy, _launcher) = hierarchy_with_instances(&[instance]);

        hierarchy
            .add(instance.into(), DesktopTarget::view(instance, view))
            .unwrap();

        let focus_target = hierarchy.instance_focus_target(instance);
        assert_eq!(focus_target, FocusableTarget::View(instance, view));
    }

    #[test]
    fn instance_focus_target_keeps_instance_without_view() {
        let instance = instance_id();
        let (hierarchy, _launcher) = hierarchy_with_instances(&[instance]);

        let focus_target = hierarchy.instance_focus_target(instance);
        assert_eq!(focus_target, FocusableTarget::Instance(instance));
    }
}
