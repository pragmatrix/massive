//! The Desktop as an event sourced user interface system.
//!
//! The presenter hierarchy is treated as an aggregate built up from the events.
//!
//! The decision to use event sourcing stems from the fact that we want to run everything as
//! incrementally as possible, because we want to add projects, etc.
//!
//! The goal here is to remove as much as possible from the specific instances into separate systems
//! and aggregates that are event driven.

mod command_dispatch;
mod commands;
mod event_forwarding;
mod focus_input;
mod focus_path_ext;
mod hierarchy_focus;
mod layout_algorithm;
mod layout_effects;
mod navigation;
mod presentation;
mod project_commands;

use anyhow::Result;
use derive_more::{Debug, From};
use std::time::Duration;

use massive_animation::Animated;
use massive_applications::{InstanceId, ViewId};
use massive_geometry::{PixelCamera, Rect, RectPx, SizePx};
use massive_layout::{IncrementalLayouter, LayoutTopology};
use massive_scene::{Location, Object, Transform};
use massive_shell::{FontManager, Scene};

pub use commands::{DesktopCommand, ProjectCommand};
use layout_algorithm::DesktopLayoutAlgorithm;
pub(crate) use layout_algorithm::place_container_children;

use crate::event_sourcing::{self, Transaction};
use crate::focus_path::FocusPath;
use crate::instance_manager::InstanceManager;
use crate::instance_presenter::InstancePresenter;
use crate::projects::{
    GroupId, GroupPresenter, LaunchProfileId, LauncherPresenter, ProjectPresenter,
};
use crate::{DesktopEnvironment, EventRouter, Map, OrderedHierarchy};

const POINTER_FEEDBACK_REENABLE_MIN_DISTANCE_PX: f64 = 24.0;
const POINTER_FEEDBACK_REENABLE_MAX_DURATION: Duration = Duration::from_millis(200);
/// This enum specifies a unique target inside the navigation and layout history.
#[derive(Debug, Clone, PartialEq, Eq, Hash, From)]
pub enum DesktopTarget {
    Desktop,

    Group(GroupId),
    Launcher(LaunchProfileId),

    Instance(InstanceId),
    View(ViewId),
}

pub type DesktopFocusPath = FocusPath<DesktopTarget>;

pub type Cmd = event_sourcing::Cmd<DesktopCommand>;

#[derive(Debug)]
pub struct DesktopSystem {
    env: DesktopEnvironment,
    fonts: FontManager,

    default_panel_size: SizePx,

    event_router: EventRouter<DesktopTarget>,
    camera: Animated<PixelCamera>,
    pointer_feedback_enabled: bool,

    #[debug(skip)]
    layouter: IncrementalLayouter<DesktopTarget, 2>,

    aggregates: Aggregates,
}

/// Aggregates are separated, so that we can control borrowing them in a more granular way.
#[derive(Debug)]
struct Aggregates {
    hierarchy: OrderedHierarchy<DesktopTarget>,

    startup_profile: Option<LaunchProfileId>,

    // presenters
    project_presenter: ProjectPresenter,
    groups: Map<GroupId, GroupPresenter>,
    launchers: Map<LaunchProfileId, LauncherPresenter>,
    instances: Map<InstanceId, InstancePresenter>,
}

impl Aggregates {
    pub fn new(
        hierarchy: OrderedHierarchy<DesktopTarget>,
        project_presenter: ProjectPresenter,
    ) -> Self {
        Self {
            hierarchy,
            startup_profile: None,
            groups: Map::default(),

            project_presenter,
            launchers: Map::default(),
            instances: Map::default(),
        }
    }
}

impl DesktopSystem {
    pub fn new(
        env: DesktopEnvironment,
        fonts: FontManager,
        default_panel_size: SizePx,
        scene: &Scene,
    ) -> Result<Self> {
        // Architecture: This is a direct requirement from the project presenter. But where does our
        // root location actually come from, shouldn't it be provided by the caller.
        let identity_matrix = Transform::IDENTITY.enter(scene);
        let location = Location::new(None, identity_matrix).enter(scene);

        let project_presenter = ProjectPresenter::new(location, scene);

        let event_router = EventRouter::new();

        let layouter = IncrementalLayouter::with_initial_reflow(DesktopTarget::Desktop);

        let system = Self {
            env,
            fonts,

            default_panel_size,

            event_router,
            camera: scene.animated(PixelCamera::default()),
            pointer_feedback_enabled: true,
            layouter,

            aggregates: Aggregates::new(OrderedHierarchy::default(), project_presenter),
        };

        Ok(system)
    }

    // Architecture: Is it really necessary to think in terms of transaction, if we update the
    // effects explicitly?
    pub fn transact(
        &mut self,
        transaction: impl Into<Transaction<DesktopCommand>>,
        scene: &Scene,
        instance_manager: &mut InstanceManager,
    ) -> Result<()> {
        for command in transaction.into() {
            self.apply_command(command, scene, instance_manager)?;
        }

        Ok(())
    }

    pub fn apply_animations(&mut self) {
        self.aggregates.project_presenter.apply_animations();
        self.aggregates
            .launchers
            .values_mut()
            .for_each(|l| l.apply_animations());
        self.aggregates
            .instances
            .values_mut()
            .for_each(|i| i.apply_animations());
    }

    pub fn is_present(&self, instance: &InstanceId) -> bool {
        self.aggregates.instances.contains_key(instance)
    }

    pub fn camera(&self) -> PixelCamera {
        self.camera.value()
    }

    pub fn cursor_visible(&self) -> bool {
        self.pointer_feedback_enabled
    }

    /// Remove the target from the hierarchy. Specific target aggregates are left
    /// untouched (they may be needed for fading out, etc.).
    pub fn remove_target(&mut self, target: &DesktopTarget) -> Result<()> {
        // Check if all components that hold reference actually removed them.
        self.event_router.notify_removed(target)?;

        let parent = self
            .aggregates
            .hierarchy
            .parent(target)
            .cloned()
            .expect("Internal error: remove_target called for root target");

        // Finally remove them.
        self.aggregates.hierarchy.remove(target)?;
        // Mark the surviving parent, not the removed node:
        // - removed nodes are ignored by incremental recompute root collection,
        // - parent refresh updates cached children and recomputes sibling placement.
        self.layouter.mark_reflow_pending(parent);
        Ok(())
    }

    fn rect(&self, target: &DesktopTarget) -> Option<Rect> {
        self.layouter.rect(target).map(|rect| {
            let rect_px: RectPx = (*rect).into();
            rect_px.into()
        })
    }
}

impl Aggregates {
    pub fn view_of_instance(&self, instance: InstanceId) -> Option<ViewId> {
        let nested = self.hierarchy.get_nested(&instance.into());
        if let [DesktopTarget::View(view)] = nested {
            Some(*view)
        } else {
            None
        }
    }
}

impl LayoutTopology<DesktopTarget> for OrderedHierarchy<DesktopTarget> {
    fn exists(&self, id: &DesktopTarget) -> bool {
        OrderedHierarchy::exists(self, id)
    }

    fn children_of(&self, id: &DesktopTarget) -> &[DesktopTarget] {
        self.get_nested(id)
    }

    fn parent_of(&self, id: &DesktopTarget) -> Option<DesktopTarget> {
        self.parent(id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn instance_id() -> InstanceId {
        Uuid::new_v4().into()
    }

    fn view_id() -> ViewId {
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
