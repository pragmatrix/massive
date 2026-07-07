//! The Desktop as an event sourced user interface system.
//!
//! The presenter hierarchy is treated as an aggregate built up from the events.
//!
//! The decision to use event sourcing stems from the fact that we want to run everything as
//! incrementally as possible, because we want to add projects, etc.
//!
//! The goal here is to remove as much as possible from the specific instances into separate systems
//! and aggregates that are event driven.

pub mod change;
mod command_dispatch;
mod commands;
mod effects;
mod event_forwarding;
mod focus_input;
mod focus_path_ext;
mod hierarchy_focus;
mod layout_algorithm;
mod layout_effects;
mod layout_state;
mod navigation;
mod presentation;
mod project_commands;
mod topology;
mod zoom_navigation;

use anyhow::{Result, bail};
use derive_more::Debug;
use log::warn;
use massive_util::CollectingVec;
use std::collections::{HashSet, VecDeque};
use std::mem;
use std::time::Duration;

use massive_animation::Animated;
use massive_applications::{InstanceId, ViewId};
use massive_geometry::{PixelCamera, SizePx};
use massive_layout::{LayoutTopology, Placement};
use massive_renderer::RenderPacing;
use massive_scene::{StageIdentityLocation, Transform};
use massive_shell::{FontManager, Scene, ShellWindow};

pub use commands::{DesktopCommand, ProjectCommand};
pub use effects::Effects;
use layout_algorithm::DesktopLayoutAlgorithm;
pub use layout_algorithm::place_container_children;
use layout_state::DesktopLayoutState;
pub(crate) use navigation::NavigationControl;

use crate::desktop_system::change::{Changes, DesktopChange};
use crate::desktop_system::effects::{DesktopEffect, MeasureSet};
use crate::desktop_system::topology::DesktopTopology;
use crate::focus_path::{FocusPath, PathResolver};
use crate::instance_manager::InstanceManager;
use crate::instance_presenter::{InstancePresenter, ViewWindowState};
use crate::projects::{
    DesktopPresenter, LaunchProfileId, LauncherPresenter, ProjectId, ProjectPresenter,
};
use crate::{DesktopEnvironment, EventRouter, Map, OrderedHierarchy};

// Require intentional mouse movement before returning pointer-first feedback after keyboard use.
const POINTER_FEEDBACK_REENABLE_MIN_DISTANCE_PX: f64 = 24.0;
const POINTER_FEEDBACK_REENABLE_MAX_DURATION: Duration = Duration::from_millis(200);
/// This enum specifies a unique target inside the navigation and layout history.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DesktopTarget {
    Desktop,

    Project(ProjectId),
    ProjectHeader(ProjectId),
    ProjectMatrix(ProjectId),
    Launcher(LaunchProfileId),

    Instance(InstanceId),
    View(ViewId),
}

impl From<ProjectId> for DesktopTarget {
    fn from(value: ProjectId) -> Self {
        Self::Project(value)
    }
}

impl From<LaunchProfileId> for DesktopTarget {
    fn from(value: LaunchProfileId) -> Self {
        Self::Launcher(value)
    }
}

impl From<InstanceId> for DesktopTarget {
    fn from(value: InstanceId) -> Self {
        Self::Instance(value)
    }
}

impl From<ViewId> for DesktopTarget {
    fn from(value: ViewId) -> Self {
        Self::View(value)
    }
}

pub type DesktopFocusPath = FocusPath<DesktopTarget>;

pub type Commands = CollectingVec<DesktopCommand>;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UserState {
    #[default]
    Focused,
    Overview(OverviewTarget),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverviewTarget {
    Visor(LaunchProfileId),
    Band(LaunchProfileId),
    Project(ProjectId),
    Desktop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FocusReason {
    InputTransition,
    StopInstanceReplacement,
    PresentInstance,
    Navigate,
    PromotePrimaryView,
}

impl FocusReason {
    pub fn resets_navigation_affinity(self) -> bool {
        match self {
            FocusReason::Navigate => false,
            FocusReason::InputTransition
            | FocusReason::StopInstanceReplacement
            | FocusReason::PresentInstance
            | FocusReason::PromotePrimaryView => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransactionEffectsMode {
    #[default]
    Normal,
    Setup,
    /// Currently, this is set when mouse buttons are pressed. I.e. the user is focusing on
    /// something specific, selecting something, etc.
    ///
    /// In this mode, the camera is prevented from moving and the launchers won't expand / collapse.
    UserGestureActive,
}

impl TransactionEffectsMode {
    pub fn animate(self) -> bool {
        match self {
            TransactionEffectsMode::Normal => true,
            TransactionEffectsMode::Setup => false,
            TransactionEffectsMode::UserGestureActive => true,
        }
    }

    pub fn permit_camera_moves(self) -> bool {
        match self {
            TransactionEffectsMode::Normal => true,
            TransactionEffectsMode::Setup => true,
            TransactionEffectsMode::UserGestureActive => false,
        }
    }
}

#[derive(Debug)]
pub struct DesktopSystem {
    env: DesktopEnvironment,
    fonts: FontManager,
    window: ShellWindow,

    default_panel_size: SizePx,

    event_router: EventRouter<DesktopTarget>,
    camera: Animated<PixelCamera>,
    user_state: UserState,
    /// Enables pointer-driven feedback (hover focus and cursor visibility).
    ///
    /// This is turned off when the user starts keyboard navigation so the pointer does not
    /// immediately steal attention, and turned back on when explicit pointer activity resumes.
    pointer_feedback_enabled: bool,
    navigation_control: NavigationControl,
    /// Focus-change measures deferred until pointer buttons are released and the camera unlocks.
    deferred_focus_launcher_measures: HashSet<LaunchProfileId>,
    /// Set when a camera move is requested while the camera is locked, so it replays once the
    /// camera unlocks (for example when a pressed mouse button is released).
    deferred_camera_move: bool,

    #[debug(skip)]
    layout_state: DesktopLayoutState,

    desktop_presenter: DesktopPresenter,
    aggregates: Aggregates,
}

pub type LauncherMap = Map<LaunchProfileId, LauncherPresenter>;

/// Aggregates are separated, so that we can control borrowing them in a more granular way.
#[derive(Debug)]
struct Aggregates {
    hierarchy: OrderedHierarchy<DesktopTarget>,

    startup_profile: Option<LaunchProfileId>,

    // presenters
    projects: Map<ProjectId, ProjectPresenter>,
    launchers: LauncherMap,
    instances: Map<InstanceId, InstancePresenter>,
}

impl Aggregates {
    pub fn new(hierarchy: OrderedHierarchy<DesktopTarget>) -> Self {
        Self {
            hierarchy,
            startup_profile: None,
            projects: Map::default(),

            launchers: Map::default(),
            instances: Map::default(),
        }
    }
}

impl DesktopSystem {
    pub fn new(
        env: DesktopEnvironment,
        fonts: FontManager,
        window: ShellWindow,
        default_panel_size: SizePx,
        scene: &Scene,
    ) -> Result<Self> {
        // Architecture: This is a direct requirement from the project presenter. But where does our
        // root location actually come from, shouldn't it be provided by the caller.
        let (_, location) = scene.stage_identity_location();

        let desktop_presenter = DesktopPresenter::new(location, scene);

        let event_router = EventRouter::new();

        let layout_state = DesktopLayoutState::new();

        let system = Self {
            env,
            fonts,
            window,

            default_panel_size,

            event_router,
            camera: scene.animated(PixelCamera::default()),
            user_state: UserState::Focused,
            pointer_feedback_enabled: true,
            navigation_control: NavigationControl::default(),
            deferred_focus_launcher_measures: Default::default(),
            deferred_camera_move: false,
            layout_state,

            desktop_presenter,
            aggregates: Aggregates::new(OrderedHierarchy::default()),
        };

        Ok(system)
    }

    // Architecture: Is it really necessary to think in terms of transaction, if we update the
    // effects explicitly?
    pub fn transact(
        &mut self,
        changes: impl Into<Changes>,
        scene: &Scene,
        instance_manager: &mut InstanceManager,
        effects_mode: impl Into<Option<TransactionEffectsMode>>,
    ) -> Result<()> {
        let changes = changes.into();
        // For live transactions the gesture mode is derived from the current pointer-button state;
        // callers only pass an explicit mode for setup.
        let effects_mode = effects_mode
            .into()
            .unwrap_or_else(|| self.live_effects_mode());

        let mut measures = MeasureSet::Empty;
        let mut user_state = self.user_state.clone();
        let focus_before = self.event_router.focused().cloned();

        let mut changes: VecDeque<DesktopChange> = changes.into_iter().collect();

        while let Some(change) = changes.pop_front() {
            let outcome = self.apply_change(change, scene, instance_manager)?;
            // TODO: I think Changes should support a DoubleEndedIterator.
            let new_changes: Vec<_> = outcome.changes.into_iter().collect();
            for new_change in new_changes.into_iter().rev() {
                changes.push_front(new_change);
            }
            measures += outcome.measures;
            user_state = outcome.user_state;
        }

        let mut effects: Effects = measures.into_iter().map(DesktopEffect::Measure).collect();

        // The camera follows the focused target, so a focus change recomputes it even when the
        // change moved no layout (pure navigation between siblings, or focusing a launcher).
        let mut update_camera = self.event_router.focused() != focus_before.as_ref();
        if self.user_state != user_state {
            self.user_state = user_state;
            update_camera = true;
        }

        // Detail: If camera moves are not allowed we assume that large visual changes aren't, too.
        // For example, focus layout effects.
        if effects_mode.permit_camera_moves() {
            self.sync_focused_launcher_anchor();
            let focus_measures = mem::take(&mut self.deferred_focus_launcher_measures);
            if !focus_measures.is_empty() {
                effects += focus_measures
                    .into_iter()
                    .map(|launcher| DesktopEffect::Measure(launcher.into()))
                    .collect::<Effects>();
            }
            // Replay a camera move that was deferred while the camera was locked (e.g. a focus
            // change that happened while a mouse button was held).
            update_camera |= mem::take(&mut self.deferred_camera_move);
        } else {
            // Camera is locked: remember a pending move so it applies once the camera unlocks (for
            // example when the pressed mouse button is released).
            self.deferred_camera_move |= update_camera;
            update_camera = false;
            // Lock camera motion immediately, including already running camera animations.
            // Ergonomics: There should probably be a function for that in Animated.
            let camera = *self.camera.value();
            self.camera.set_immediately(camera);
        }

        // This should probably be a function call and does not need to be an effect anymore.
        if update_camera {
            effects += DesktopEffect::UpdateCamera;
        }

        // Commands emit their own targeted `Measure` effects for the subtrees they change, and a
        // focus change emits `UpdateCamera` directly (see the `focus_before` comparison above), so
        // no root measure is needed here.
        self.run_effects_to_completion(effects_mode, effects)?;

        // Sync the window state (title, cursor) from the focused view after all effects settle.
        self.apply_focused_view_window_state()?;

        Ok(())
    }

    pub fn apply_animations(&mut self) {
        let launcher_instance_ids: Vec<_> = self
            .aggregates
            .launchers
            .keys()
            .copied()
            .map(|launcher_id| {
                (
                    launcher_id,
                    self.aggregates.hierarchy.launcher_instances(launcher_id),
                )
            })
            .collect();

        for (launcher_id, child_instances) in launcher_instance_ids {
            self.aggregates
                .launchers
                .get_mut(&launcher_id)
                .expect("Launcher missing")
                .apply_animations(&mut self.aggregates.instances, &child_instances);
        }

        for project in self.aggregates.projects.values_mut() {
            project.apply_animations();
        }

        let pointer_focus = if self.pointer_feedback_enabled {
            self.event_router.pointer_focus().cloned()
        } else {
            None
        };
        // Hover must track animated instance transforms, not just transaction-time layout updates.
        self.sync_hover_rect_to_pointer_path(pointer_focus.as_ref());
    }

    pub fn is_present(&self, instance: &InstanceId) -> bool {
        self.aggregates.instances.contains_key(instance)
    }

    pub fn camera(&mut self) -> &PixelCamera {
        self.camera.value()
    }

    pub fn is_cursor_visible(&self) -> bool {
        self.pointer_feedback_enabled
    }

    pub fn any_buttons_pressed(&self) -> bool {
        self.event_router.any_buttons_pressed()
    }

    /// The effects mode for a live (non-setup) transaction, derived from pointer-button state.
    fn live_effects_mode(&self) -> TransactionEffectsMode {
        if self.any_buttons_pressed() {
            TransactionEffectsMode::UserGestureActive
        } else {
            TransactionEffectsMode::Normal
        }
    }

    pub fn set_instance_pacing(&mut self, instance: InstanceId, pacing: RenderPacing) {
        if let Some(instance_presenter) = self.aggregates.instances.get_mut(&instance) {
            instance_presenter.pacing = pacing;
        } else {
            warn!("Setting pacing on an unknown instance");
        }
    }

    pub fn effective_pacing(&self) -> RenderPacing {
        if self
            .aggregates
            .instances
            .values()
            .any(|instance| instance.pacing == RenderPacing::Smooth)
        {
            RenderPacing::Smooth
        } else {
            RenderPacing::Fast
        }
    }

    pub fn focused_view_window_state(&self) -> Result<Option<ViewWindowState>> {
        let Some(focused) = self.event_router.focused() else {
            return Ok(None);
        };

        let focused_path = self.path_of(Some(focused));
        let Some(instance) = focused_path.instance() else {
            return Ok(None);
        };
        let Some(instance_presenter) = self.aggregates.instances.get(&instance) else {
            bail!("Focused instance has no presenter");
        };

        let Some(view) = self.aggregates.view_of_instance(instance) else {
            return Ok(None);
        };

        instance_presenter
            .view_window_state(view)
            .cloned()
            .map(Some)
    }

    /// Remove the target from the hierarchy. Specific target aggregates are left
    /// untouched (they may be needed for fading out, etc.).
    fn remove_target(&mut self, target: &DesktopTarget) -> Result<MeasureSet> {
        // Check if all components that hold reference actually removed them.
        self.event_router.notify_removed(target)?;

        let parent = self
            .aggregates
            .hierarchy
            .parent(target)
            .cloned()
            .expect("Internal error: remove_target called for root target");

        // Explicitly invalidate the layout cache for the removed subtree.
        self.layout_state
            .remove_subtree(target, &self.aggregates.hierarchy);

        // Finally remove them.
        self.aggregates.hierarchy.remove(target)?;
        // Mark the surviving parent, not the removed node:
        // - removed nodes are ignored by incremental recompute root collection,
        // - parent refresh updates cached children and recomputes sibling placement.
        Ok(parent.into())
    }

    fn placement(&self, target: &DesktopTarget) -> Placement<Transform, 2> {
        self.layout_state
            .absolute_placement(target, &self.aggregates.hierarchy)
    }

    pub(super) fn focused_path(&self) -> DesktopFocusPath {
        self.path_of(self.event_router.focused())
    }

    pub(super) fn path_of<'a>(
        &'a self,
        target: impl Into<Option<&'a DesktopTarget>>,
    ) -> DesktopFocusPath {
        self.aggregates.hierarchy.resolve_path(target.into())
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

    /// Returns the direct children of `id`, or `[]` when the target is not present.
    fn children_of(&self, id: &DesktopTarget) -> &[DesktopTarget] {
        self.get_nested(id)
    }

    fn parent_of(&self, id: &DesktopTarget) -> Option<&DesktopTarget> {
        self.parent(id)
    }
}
