//! The Desktop as an event sourced user interface system.
//!
//! The presenter hierarchy is treated as an aggregate built up from the events.
//!
//! The decision to use event sourcing stems from the fact that we want to run everything as
//! incrementally as possible, because we want to add projects, etc.
//!
//! The goal here is to remove as much as possible from the specific instances into separate systems
//! and aggregates that are event driven.

mod commands;
mod event_forwarding;
mod focus_input;
mod focus_path_ext;
mod hierarchy_focus;
mod layout_algorithm;
mod navigation;
mod presentation;

use anyhow::{Result, anyhow};
use derive_more::{Debug, From};
use log::warn;
use std::collections::HashSet;
use std::time::Duration;

use massive_animation::{Animated, Interpolation};
use massive_applications::{CreationMode, InstanceId, ViewId, ViewRole};
use massive_geometry::{PixelCamera, PointPx, Rect, RectPx, SizePx};
use massive_layout::{IncrementalLayouter, LayoutTopology, Rect as LayoutRect};
use massive_scene::{Location, Object, Transform};
use massive_shell::{FontManager, Scene};

pub use commands::{DesktopCommand, ProjectCommand};
use layout_algorithm::DesktopLayoutAlgorithm;
pub(crate) use layout_algorithm::place_container_children;

use crate::event_sourcing::{self, Transaction};
use crate::focus_path::{FocusPath, PathResolver};
use crate::instance_manager::InstanceManager;
use crate::instance_presenter::{InstancePresenter, STRUCTURAL_ANIMATION_DURATION};
use crate::projects::{
    GroupId, GroupPresenter, LaunchProfileId, LauncherInstanceLayoutInput,
    LauncherInstanceLayoutTarget, LauncherPresenter, ProjectPresenter,
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

    // Architecture: The current focus is part of the system, so DesktopInteraction should probably be embedded here.
    fn apply_command(
        &mut self,
        command: DesktopCommand,
        scene: &Scene,
        instance_manager: &mut InstanceManager,
    ) -> Result<()> {
        match command {
            DesktopCommand::StartInstance {
                launcher,
                parameters,
            } => {
                // Feature: Support starting non-primary applications.
                let application = self
                    .env
                    .applications
                    .get_named(&self.env.primary_application)
                    .ok_or(anyhow!("Internal error, application not registered"))?;

                let instance =
                    instance_manager.spawn(application, CreationMode::New(parameters))?;

                // Robustness: Should this be a real, logged event?
                // Architecture: Better to start up the primary directly, so that we can remove the PresentInstance command?
                self.apply_command(
                    DesktopCommand::PresentInstance { launcher, instance },
                    scene,
                    instance_manager,
                )
            }

            DesktopCommand::StopInstance(instance) => {
                // Remove the instance from the focus first.
                //
                // Detail: This causes an unfocus event sent to the instance's view which may
                // unexpected while teardown.

                let target = DesktopTarget::Instance(instance);
                let replacement_focus = self
                    .aggregates
                    .hierarchy
                    .resolve_replacement_focus_for_stopping_instance(
                        self.event_router.focused(),
                        instance,
                    );

                if let Some(replacement_focus) = replacement_focus {
                    self.set_keyboard_focus_without_command(
                        Some(&replacement_focus),
                        instance_manager,
                    )?;
                }

                self.unfocus_pointer_if_path_contains(&target, instance_manager)?;

                // This might fail if StopInstance gets triggered with an instance that ended in
                // itself (shouldn't the instance_manager keep it until we finally free it).
                if let Err(e) = instance_manager.request_shutdown(instance) {
                    warn!("Failed to shutdown instance, it may be gone already: {e}");
                };

                // We hide the instance as soon we request a shutdown so that they can't be in the
                // navigation tree anymore.
                self.hide_instance(instance)?;

                Ok(())
            }

            DesktopCommand::PresentInstance { launcher, instance } => {
                let focused = self.event_router.focused();
                let focused_path = self.aggregates.hierarchy.resolve_path(focused);

                let originating_from = focused_path.instance();

                let insertion_index =
                    self.present_instance(launcher, originating_from, instance, scene)?;

                let instance_target = DesktopTarget::Instance(instance);

                // Add this instance to the hierarchy.
                self.aggregates.hierarchy.insert_at(
                    launcher.into(),
                    insertion_index,
                    instance_target.clone(),
                )?;
                self.layouter
                    .mark_reflow_pending(DesktopTarget::Launcher(launcher));

                // Focus it.
                let transitions = self.event_router.focus(&instance_target);
                let cmd = self.forward_event_transitions(transitions, instance_manager)?;
                assert!(cmd.is_none());
                Ok(())
            }

            DesktopCommand::PresentView(instance, creation_info) => {
                self.present_view(instance, &creation_info)?;

                let focused = self.event_router.focused();
                // If this instance is currently focused and the new view is primary, make it
                // foreground so that the view is focused.
                if matches!(focused, Some(DesktopTarget::Instance(i)) if *i == instance)
                    && creation_info.role == ViewRole::Primary
                {
                    let cmd =
                        self.focus(&DesktopTarget::View(creation_info.id), instance_manager)?;
                    assert!(cmd.is_none())
                }

                Ok(())
            }
            DesktopCommand::HideView(view_path) => self.hide_view(view_path),

            DesktopCommand::Project(project_command) => {
                self.apply_project_command(project_command, scene)
            }

            DesktopCommand::ZoomOut => {
                if let Some(focused) = self.event_router.focused()
                    && let Some(parent) = self.aggregates.hierarchy.parent(focused)
                {
                    assert!(self.focus(&parent.clone(), instance_manager)?.is_none());
                }
                Ok(())
            }
            DesktopCommand::Navigate(direction) => {
                if let Some(focused) = self.event_router.focused()
                    && let Some(candidate) = self.locate_navigation_candidate(focused, direction)
                {
                    assert!(self.focus(&candidate, instance_manager)?.is_none());
                }
                Ok(())
            }
        }
    }

    fn apply_project_command(&mut self, command: ProjectCommand, scene: &Scene) -> Result<()> {
        match command {
            ProjectCommand::AddLaunchGroup {
                parent,
                id,
                properties,
            } => {
                let parent = parent.map(|p| p.into()).unwrap_or(DesktopTarget::Desktop);
                self.aggregates.hierarchy.add(parent.clone(), id.into())?;
                self.aggregates
                    .groups
                    .insert(id, GroupPresenter::new(properties))?;
                self.layouter.mark_reflow_pending(parent);
            }
            ProjectCommand::RemoveLaunchGroup(group) => {
                self.remove_target(&group.into())?;
                self.aggregates.groups.remove(&group)?;
            }
            ProjectCommand::AddLauncher { group, id, profile } => {
                let presenter = LauncherPresenter::new(
                    self.aggregates.project_presenter.location.clone(),
                    id,
                    profile,
                    massive_geometry::Rect::default(),
                    scene,
                    &mut self.fonts.lock(),
                );
                self.aggregates.launchers.insert(id, presenter)?;

                self.aggregates.hierarchy.add(group.into(), id.into())?;
                self.layouter
                    .mark_reflow_pending(DesktopTarget::Group(group));
            }
            ProjectCommand::RemoveLauncher(id) => {
                let target = DesktopTarget::Launcher(id);
                self.remove_target(&target)?;

                self.aggregates.launchers.remove(&id)?;
            }
            ProjectCommand::SetStartupProfile(launch_profile_id) => {
                self.aggregates.startup_profile = launch_profile_id
            }
        }

        Ok(())
    }

    /// Update layout changes and the camera position.
    pub fn update_effects(&mut self, animate: bool, permit_camera_moves: bool) -> Result<()> {
        // Layout & apply rects.
        let algorithm = DesktopLayoutAlgorithm {
            aggregates: &self.aggregates,
            default_panel_size: self.default_panel_size,
        };
        let changed = self
            .layouter
            .recompute(&self.aggregates.hierarchy, &algorithm, PointPx::origin())
            .changed;
        self.apply_layout_changes(changed, animate);

        // Camera

        if permit_camera_moves && let Some(focused) = self.event_router.focused() {
            let camera = self.camera_for_focus(focused);
            if let Some(camera) = camera {
                if animate {
                    self.camera.animate_if_changed(
                        camera,
                        STRUCTURAL_ANIMATION_DURATION,
                        Interpolation::CubicOut,
                    );
                } else {
                    self.camera.set_immediately(camera);
                }
            }
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

    fn apply_layout_changes(
        &mut self,
        changed: Vec<(DesktopTarget, LayoutRect<2>)>,
        animate: bool,
    ) {
        let mut launchers_to_relayout: HashSet<LaunchProfileId> = HashSet::new();

        for (id, layout_rect) in changed {
            let rect_px: RectPx = layout_rect.into();
            let rect: Rect = rect_px.into();

            match id {
                DesktopTarget::Desktop => {}
                DesktopTarget::Instance(instance_id) => {
                    if let Some(launcher_id) = self.instance_launcher(instance_id) {
                        launchers_to_relayout.insert(launcher_id);
                    }
                }
                DesktopTarget::Group(group_id) => {
                    self.aggregates
                        .groups
                        .get_mut(&group_id)
                        .expect("Missing group")
                        .rect = rect;
                }
                DesktopTarget::Launcher(launcher_id) => {
                    launchers_to_relayout.insert(launcher_id);

                    self.aggregates
                        .launchers
                        .get_mut(&launcher_id)
                        .expect("Launcher missing")
                        .set_rect(rect, animate);
                }
                DesktopTarget::View(..) => {
                    // Robustness: Support resize here?
                }
            }
        }

        for launcher_id in launchers_to_relayout {
            self.apply_launcher_instance_layout(launcher_id, animate);
        }
    }

    fn instance_launcher(&self, instance_id: InstanceId) -> Option<LaunchProfileId> {
        let instance_target = DesktopTarget::Instance(instance_id);
        match self.aggregates.hierarchy.parent(&instance_target) {
            Some(DesktopTarget::Launcher(id)) => Some(*id),
            _ => None,
        }
    }

    fn apply_launcher_instance_layout(&mut self, launcher_id: LaunchProfileId, animate: bool) {
        let launcher_target = DesktopTarget::Launcher(launcher_id);
        let instance_inputs: Vec<LauncherInstanceLayoutInput> = self
            .aggregates
            .hierarchy
            .get_nested(&launcher_target)
            .iter()
            .filter_map(|target| match target {
                DesktopTarget::Instance(instance_id) => {
                    let instance_target = DesktopTarget::Instance(*instance_id);
                    let rect_px: RectPx =
                        (*self.layouter.rect(&instance_target).unwrap_or_else(|| {
                            panic!("Internal error: Missing layout rect for {instance_target:?}")
                        }))
                        .into();

                    Some(LauncherInstanceLayoutInput {
                        instance_id: *instance_id,
                        rect: rect_px,
                    })
                }
                _ => None,
            })
            .collect();

        let focused_instance = self
            .aggregates
            .hierarchy
            .resolve_path(self.event_router.focused())
            .instance();
        let layouts: Vec<LauncherInstanceLayoutTarget> = self
            .aggregates
            .launchers
            .get(&launcher_id)
            .expect("Launcher missing")
            .compute_instance_layout_targets(&instance_inputs, focused_instance);

        // Apply transform updates so presenter animations can interpolate to the new cylinder state.
        for layout in layouts {
            self.aggregates
                .instances
                .get_mut(&layout.instance_id)
                .expect("Instance missing")
                .set_layout(layout.rect, layout.layout_transform, animate);
        }
    }

    fn apply_launcher_layout_for_focus_change(
        &mut self,
        from: Option<DesktopTarget>,
        to: Option<DesktopTarget>,
        animate: bool,
    ) {
        // Architecture: I don't like this before/after focus comparison test.
        // No focus transition means there is no cylinder rotation target change.
        if from == to {
            return;
        }

        // Update at most the launchers touched by the old/new focus targets.
        let mut launchers_to_update: HashSet<LaunchProfileId> = HashSet::new();
        for target in [from.as_ref(), to.as_ref()] {
            if let Some(launcher_id) = self.focus_target_launcher_for_layout(target) {
                launchers_to_update.insert(launcher_id);
            }
        }

        // Recompute launcher transforms immediately so the focus move animates right away.
        for launcher_id in launchers_to_update {
            self.apply_launcher_instance_layout(launcher_id, animate);
        }
    }

    fn focus_target_launcher_for_layout(
        &self,
        target: Option<&DesktopTarget>,
    ) -> Option<LaunchProfileId> {
        // Resolve from any focus target (instance/view/etc.) to its owning instance.
        let target = target?;
        let focused_path = self.aggregates.hierarchy.resolve_path(Some(target));
        let focused_instance = focused_path.instance()?;
        let launcher_id = self.instance_launcher(focused_instance)?;
        let instance_count = self
            .aggregates
            .hierarchy
            .get_nested(&DesktopTarget::Launcher(launcher_id))
            .len();

        self.aggregates
            .launchers
            .get(&launcher_id)
            .filter(|launcher| launcher.should_relayout_on_focus_change(instance_count))
            .map(|_| launcher_id)
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
