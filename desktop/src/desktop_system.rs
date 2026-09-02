//! The Desktop as an event sourced user interface system.
//!
//! The presenter hierarchy is treated as an aggregate built up from the events.
//!
//! The decision to use event sourcing stems from the fact that we want to run everything as
//! incrementally as possible, because we want to add projects, etc.
//!
//! The goal here is to remove as much as possible from the specific instances into separate systems
//! and aggregates that are event driven.

mod camera_presentation;
pub mod change;
mod change_surface;
mod command_dispatch;
mod commands;
mod effects;
mod event_forwarding;
mod focus_depth_indicator;
mod focus_input;
mod focus_path_ext;
mod fullscreen;
mod hierarchy_focus;
mod layout_algorithm;
mod layout_effects;
mod layout_state;
mod navigation;
mod presentation;
mod topology;

use std::collections::{HashSet, VecDeque};
use std::mem;
use std::time::Instant;

use anyhow::Result;
use derive_more::Debug;
use log::warn;

use massive_animation::MovementRuntime;
use massive_applications::{InstanceId, ViewId};
use massive_geometry::{PixelCamera, SizePx};
use massive_layout::{LayoutTopology, Placement};
use massive_renderer::RenderPacing;
use massive_scene::{IdentityLocation, Transform};
use massive_shell::{FontManager, Frame, Scene};
use massive_util::CollectingVec;

use camera_presentation::{CameraPresentation, CameraPresentationMode};
use change::{Changes, DesktopChange};
use effects::DesktopEffect;
use focus_depth_indicator::FocusDepthIndicatorPresenter;
use layout_algorithm::DesktopLayoutAlgorithm;
use layout_state::DesktopLayoutState;
use navigation::NavigationControl;

pub(crate) use commands::{DesktopCommand, ProjectCommand};
pub(crate) use effects::Effects;
pub(crate) use fullscreen::fullscreen_scale;
pub(crate) use layout_algorithm::place_container_children;

use crate::desktop_presenter::DesktopPresenter;
use crate::desktop_system::change_surface::{ChangeSurface, TargetSet};
use crate::focus_path::{FocusPath, PathResolver};
use crate::instance_manager::InstanceManager;
use crate::instance_presenter::{InstancePresenter, ViewWindowState};
use crate::projects::{LaunchProfileId, LauncherPresenter, ProjectId, ProjectPresenter};
use crate::{DesktopEnvironment, EventRouter, Map, MatrixPositions, OrderedHierarchy};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
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

/// What is the user currently focusing on.
///
/// As a general rule: The focus depth is always selectable by the user, but the implementation by
/// the system is optional and depends on the currently focused target.
///
/// The system should show when the focus depth is changed, so that the user knows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, strum::EnumCount, strum::FromRepr)]
#[repr(u8)]
pub enum FocusDepth {
    InstanceFullScreen,
    #[default]
    Instance,
    Launcher,
    Row,
    Project,
    Desktop,
}

impl FocusDepth {
    pub fn zoom_in(self) -> Option<Self> {
        Self::from_repr((self as u8).checked_sub(1)?)
    }

    pub fn zoom_out(self) -> Option<Self> {
        Self::from_repr((self as u8).checked_add(1)?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardFocusReason {
    InputTransition,
    StopInstanceReplacement,
    PresentInstance,
    Navigate,
    PromotePrimaryView,
}

impl KeyboardFocusReason {
    pub fn resets_navigation_affinity(self) -> bool {
        match self {
            KeyboardFocusReason::Navigate => false,
            KeyboardFocusReason::InputTransition
            | KeyboardFocusReason::StopInstanceReplacement
            | KeyboardFocusReason::PresentInstance
            | KeyboardFocusReason::PromotePrimaryView => true,
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
    pub fn permit_animations(self) -> bool {
        match self {
            TransactionEffectsMode::Normal => true,
            TransactionEffectsMode::Setup => false,
            TransactionEffectsMode::UserGestureActive => true,
        }
    }

    fn camera_presentation_mode(self) -> CameraPresentationMode {
        match self {
            TransactionEffectsMode::Normal => CameraPresentationMode::Animate,
            TransactionEffectsMode::Setup => CameraPresentationMode::Snap,
            TransactionEffectsMode::UserGestureActive => CameraPresentationMode::Freeze,
        }
    }
}

#[derive(Debug)]
pub struct DesktopSystem {
    env: DesktopEnvironment,
    fonts: FontManager,

    default_panel_size: SizePx,

    event_router: EventRouter<DesktopTarget>,
    camera: CameraPresentation,
    focus_depth: FocusDepth,
    navigation_control: NavigationControl,
    /// Focus-change measures deferred until pointer buttons are released and the camera unlocks.
    deferred_focus_launcher_measures: HashSet<LaunchProfileId>,

    #[debug(skip)]
    layout_state: DesktopLayoutState,

    pub focus_depth_indicator: FocusDepthIndicatorPresenter,
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
    matrix_positions: MatrixPositions,
    instances: Map<InstanceId, InstancePresenter>,
}

impl Aggregates {
    pub fn new(hierarchy: OrderedHierarchy<DesktopTarget>) -> Self {
        Self {
            hierarchy,
            startup_profile: None,
            projects: Map::default(),

            launchers: Map::default(),
            matrix_positions: MatrixPositions::default(),
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
        movement_runtime: &mut MovementRuntime,
    ) -> Result<Self> {
        // Architecture: This is a direct requirement from the project presenter. But where does our
        // root location actually come from, shouldn't it be provided by the caller.
        let location = scene.identity_location().enter(scene).location;

        let desktop_presenter = DesktopPresenter::new(location, scene, movement_runtime);
        let focus_depth_indicator =
            FocusDepthIndicatorPresenter::new(scene, &mut fonts.lock(), movement_runtime);

        let event_router = EventRouter::new();

        let layout_state = DesktopLayoutState::new();

        let system = Self {
            env,
            fonts,

            default_panel_size,

            event_router,
            camera: CameraPresentation::new(PixelCamera::default()),
            focus_depth: FocusDepth::default(),
            navigation_control: NavigationControl::default(),
            deferred_focus_launcher_measures: Default::default(),
            layout_state,

            focus_depth_indicator,
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
        frame: &mut Frame,
        instance_manager: &mut InstanceManager,
        effects_mode: impl Into<Option<TransactionEffectsMode>>,
        window_size: SizePx,
    ) -> Result<()> {
        let changes = changes.into();
        let previous_focus_depth = self.focus_depth;
        // For live transactions the gesture mode is derived from the current pointer-button state;
        // callers only pass an explicit mode for setup.
        let effects_mode = effects_mode
            .into()
            .unwrap_or_else(|| self.live_effects_mode());

        // Run changes to completion and combine everything into a `ChangeSurface`.

        let mut change_surface = ChangeSurface::default();
        {
            let mut changes: VecDeque<DesktopChange> = changes.into_iter().collect();
            while let Some(change) = changes.pop_front() {
                let output = self.apply_change(change, frame, instance_manager)?;
                // TODO: I think Changes should support a DoubleEndedIterator.
                for new_change in output
                    .changes
                    .into_iter()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                {
                    changes.push_front(new_change);
                }
                change_surface.combine(output.surface);
            }
        }

        // Collect deferred measures if the camera can be moved.

        // Detail: If camera moves are not allowed we assume that large visual changes aren't, too.
        // For example, focus layout effects.
        //
        // Design: may replace deferred_* with a ChangeSurface (a "deferred" ChangeSurface?).
        let camera_mode = effects_mode.camera_presentation_mode();
        if camera_mode.permit_camera_moves() {
            self.sync_focused_launcher_anchor();
            change_surface.size_invalid += mem::take(&mut self.deferred_focus_launcher_measures)
                .into_iter()
                .map(DesktopTarget::Launcher)
                .collect::<TargetSet>();
        }

        // Only keep the `ChangeSurface` targets that match against the final topology.
        //
        // A later change in the same transaction may remove a target scheduled by an earlier
        // change.
        change_surface.retain(|target| self.aggregates.hierarchy.exists(target));

        let update_camera = change_surface.camera_invalid();

        // Convert the change surface to effects.
        let effects = convert_change_surface_to_effects(change_surface);

        // Window size is needed to resolve layout and camera focus for presenters that
        // must fit into the window.
        self.run_effects_to_completion(effects_mode, effects, window_size, instance_manager)?;

        // Resolve camera intent after all effects were run, when all placements are final.
        if update_camera {
            let desired = self.resolve_desired_camera(window_size);
            self.camera.set_desired(desired);
        }

        let animation_time = frame.animation_time();
        self.camera.synchronize(animation_time, frame, camera_mode);

        if effects_mode != TransactionEffectsMode::Setup && self.focus_depth != previous_focus_depth
        {
            self.focus_depth_indicator
                .show(self.focus_depth, animation_time);
        }

        // Update the hover target.
        {
            let hover_target = self
                .event_router
                .pointer_focus()
                .or_else(|| self.event_router.keyboard_focus());

            // Sync the hover rect.
            self.sync_hover_with_target(hover_target.cloned().as_ref());
        }

        Ok(())
    }

    pub fn is_present(&self, instance: &InstanceId) -> bool {
        self.aggregates.instances.contains_key(instance)
    }

    pub fn camera(&mut self, instant: Instant) -> &PixelCamera {
        self.camera.proceed(instant)
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

    pub fn animating_instances(&self) -> impl Iterator<Item = InstanceId> + '_ {
        self.aggregates
            .instances
            .iter()
            .filter(|(_, instance)| instance.pacing == RenderPacing::Smooth)
            .map(|(id, _)| *id)
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
        let Some(focused) = self.event_router.keyboard_focus() else {
            return Ok(None);
        };

        let focused_path = self.path_of(Some(focused));
        let Some(instance) = focused_path.instance() else {
            return Ok(None);
        };
        let Some(instance_presenter) = self.aggregates.instances.get(&instance) else {
            panic!("Focused instance has no presenter");
        };

        let Some(view) = self.aggregates.view_of_instance(instance) else {
            return Ok(None);
        };

        Ok(Some(instance_presenter.view_window_state(view)?.clone()))
    }

    /// Remove the target from the hierarchy. Specific target aggregates are left
    /// untouched (they may be needed for fading out, etc.).
    fn remove_target(&mut self, target: &DesktopTarget) -> Result<DesktopTarget> {
        // Check if all components that hold reference actually removed them.
        self.event_router.notify_removed(target)?;

        let parent = self
            .aggregates
            .hierarchy
            .parent(target)
            .cloned()
            .expect("Internal error: remove_target called for root target");

        // Evict the removed subtree's cache entries. Not needed for recompute correctness (the
        // parent remeasure below reads only the surviving children); this just prevents stale
        // entries from leaking, since this is their only eviction path.
        self.layout_state
            .remove_subtree(target, &self.aggregates.hierarchy);

        // Finally remove them.
        self.aggregates.hierarchy.remove(target)?;
        // Mark the surviving parent, not the removed node:
        // - removed nodes are ignored by incremental recompute root collection,
        // - parent refresh updates cached children and recomputes sibling placement.
        Ok(parent)
    }

    fn placement(&self, target: &DesktopTarget) -> Placement<Transform, 2> {
        self.layout_state
            .absolute_placement(target, &self.aggregates.hierarchy)
    }

    pub(super) fn focused_path(&self) -> DesktopFocusPath {
        self.path_of(self.event_router.keyboard_focus())
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

fn convert_change_surface_to_effects(surface: ChangeSurface) -> Effects {
    surface
        .size_invalid
        .into_iter()
        .map(DesktopEffect::Measure)
        .collect()
}
