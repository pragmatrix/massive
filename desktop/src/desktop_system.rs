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
mod focus_input;
mod focus_path_ext;
mod fullscreen;
mod hierarchy_focus;
mod key_delivery;
mod keyboard_shortcuts;
mod layout_algorithm;
mod layout_effects;
mod layout_state;
mod navigation;
mod presentation;
mod topology;
mod zoom_level_indicator;

use std::collections::{HashSet, VecDeque};
use std::mem;

use anyhow::Result;
use derive_more::Debug;
use log::warn;

use massive_applications::prelude::*;
use massive_applications::{InstanceId, ViewId};
use massive_geometry::{PixelCamera, SizePx, Transform};
use massive_layout::{LayoutTopology, Placement};
use massive_renderer::RenderPacing;
use massive_scene::prelude::identity_location;
use massive_scene::{Handle, Location};
use massive_util::CollectingVec;

use camera_presentation::{CameraPresentation, CameraPresentationMode};
use change::{Changes, DesktopChange, DesktopSystemEffect};
use effects::DesktopEffect;
use layout_algorithm::DesktopLayoutAlgorithm;
use layout_state::DesktopLayoutState;
use navigation::NavigationControl;
use zoom_level_indicator::ZoomLevelIndicatorPresenter;

pub(crate) use commands::{DesktopCommand, ProjectCommand};
pub(crate) use effects::Effects;
pub(crate) use fullscreen::{fullscreen_scale, view_size};
pub(crate) use key_delivery::{KeyContext, KeyHandler, KeyInput, KeyOutcome};
pub(crate) use keyboard_shortcuts::Shortcut;
pub(crate) use layout_algorithm::{instance_extent, place_container_children};
pub(crate) use massive_applications::SlotShift;

use crate::desktop_presenter::DesktopPresenter;
use crate::desktop_system::change_surface::{ChangeSurface, TargetSet};
use crate::focus_path::{FocusPath, PathResolver};
use crate::instance_manager::InstanceManager;
use crate::instance_presenter::{InstancePresenter, InstanceTitleBarMetrics, ViewWindowState};
use crate::projects::FullScreenMode;
use crate::projects::{
    LaunchProfileId, LauncherPresenter, ProjectId, ProjectPresenter, RuntimeConfiguration,
};
use crate::window_state::WindowState;
use crate::{DesktopEnvironment, EventRouter, Map, OrderedHierarchy};
/// This enum specifies a unique target inside the navigation and layout history.
///
/// `Desktop` is the hierarchy's virtual root: it is never inserted explicitly and
/// has no presenter — it only appears as the parent key under which the root
/// project's target is added.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DesktopTarget {
    Desktop,
    Project(ProjectId),
    ProjectHeader(ProjectId),
    ProjectMatrix(ProjectId),
    Launcher(LaunchProfileId),

    Instance(InstanceId),
    /// The bar above an instance's view (ADR 0019). Selects its instance.
    InstanceTitleBar(InstanceId),
    View(ViewId),
}

impl DesktopTarget {
    /// An instance's title bar stands for its instance (ADR 0019); every other target stays as it
    /// is.
    pub fn title_bar_as_instance(self) -> Self {
        match self {
            Self::InstanceTitleBar(instance) => Self::Instance(instance),
            target => target,
        }
    }

    /// Whether the target is an instance or part of one (title bar, view).
    pub fn is_instance_target(&self) -> bool {
        matches!(
            self,
            Self::Instance(_) | Self::InstanceTitleBar(_) | Self::View(_)
        )
    }

    /// The project a project-level target stands for; `Desktop` stands for the root project.
    pub fn stands_for_project(&self) -> Option<ProjectId> {
        match self {
            Self::Project(project)
            | Self::ProjectHeader(project)
            | Self::ProjectMatrix(project) => Some(*project),
            Self::Desktop => Some(ProjectId::ROOT),
            Self::Launcher(_) | Self::Instance(_) | Self::InstanceTitleBar(_) | Self::View(_) => {
                None
            }
        }
    }

    /// A project target receives no text input, so `Enter` without `Cmd` enters it (ADR 0018).
    pub fn enters_on_plain_enter(&self) -> bool {
        matches!(
            self,
            Self::Project(_) | Self::ProjectHeader(_) | Self::ProjectMatrix(_)
        )
    }
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

/// What the camera frames, relative to the keyboard-focused target and the project whose matrix
/// holds its slot (ADR 0018). Ordered from the outermost to the innermost frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum ZoomLevel {
    /// The project whose matrix holds the focused target's slot.
    Project,
    /// The matrix row of the focused target's slot.
    Row,
    /// The focused target's slot.
    Slot,
    /// The focused target itself.
    #[default]
    Focus,
}

impl ZoomLevel {
    /// The next inner level, `None` at `Focus`.
    pub fn zoom_in(self, instance_target: bool) -> Option<Self> {
        match self.normalized(instance_target) {
            ZoomLevel::Project => Some(ZoomLevel::Row),
            ZoomLevel::Row if instance_target => Some(ZoomLevel::Slot),
            ZoomLevel::Row | ZoomLevel::Slot => Some(ZoomLevel::Focus),
            ZoomLevel::Focus => None,
        }
    }

    /// The next outer level, `None` at `Project`.
    pub fn zoom_out(self, instance_target: bool) -> Option<Self> {
        match self.normalized(instance_target) {
            ZoomLevel::Focus if instance_target => Some(ZoomLevel::Slot),
            ZoomLevel::Focus | ZoomLevel::Slot => Some(ZoomLevel::Row),
            ZoomLevel::Row => Some(ZoomLevel::Project),
            ZoomLevel::Project => None,
        }
    }

    /// On a target that is not an instance, `Slot` frames the same rect as `Focus` and reads as
    /// `Focus`.
    pub fn normalized(self, instance_target: bool) -> Self {
        match self {
            ZoomLevel::Slot if !instance_target => ZoomLevel::Focus,
            level => level,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ZoomLevelState {
    zoom_level: ZoomLevel,
    keyboard_focus: Option<DesktopTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardFocusReason {
    InputTransition,
    StopInstanceReplacement,
    PresentInstance,
    Navigate,
    PromotePrimaryView,
    /// Zooming moved focus across a project boundary (ADR 0018).
    Zoom,
}

impl KeyboardFocusReason {
    pub fn resets_navigation_affinity(self) -> bool {
        match self {
            KeyboardFocusReason::Navigate => false,
            KeyboardFocusReason::InputTransition
            | KeyboardFocusReason::StopInstanceReplacement
            | KeyboardFocusReason::PresentInstance
            | KeyboardFocusReason::PromotePrimaryView
            | KeyboardFocusReason::Zoom => true,
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

/// Host-facing effects emitted by a completed desktop transaction.
#[derive(Debug)]
pub struct TransactionOutput {
    /// Effects that require capabilities owned by the desktop host.
    pub effects: Vec<DesktopSystemEffect>,
}

#[derive(Debug)]
pub struct DesktopSystem {
    env: DesktopEnvironment,

    default_panel_size: SizePx,
    /// The window state, committed by `DesktopChange::WindowResized` — the
    /// constructor seeds it from the instance extent, the default panel size plus the title bar
    /// (ADR 0014: the spawn path reads the inner size to seed a fullscreen instance's application
    /// canvas).
    window_state: WindowState,

    event_router: EventRouter<DesktopTarget>,

    camera: CameraPresentation,
    zoom_level: ZoomLevel,
    navigation_control: NavigationControl,
    /// Focus-change measures deferred until pointer buttons are released and the camera unlocks.
    deferred_focus_launcher_measures: HashSet<LaunchProfileId>,

    #[debug(skip)]
    layout_state: DesktopLayoutState,

    zoom_level_indicator: ZoomLevelIndicatorPresenter,
    desktop_presenter: DesktopPresenter,
    aggregates: Aggregates,
}

pub type LauncherMap = Map<LaunchProfileId, LauncherPresenter>;
pub type ProjectMap = Map<ProjectId, ProjectPresenter>;
pub type InstanceMap = Map<InstanceId, InstancePresenter>;

/// Aggregates are separated, so that we can control borrowing them in a more granular way.
#[derive(Debug)]
struct Aggregates {
    hierarchy: OrderedHierarchy<DesktopTarget>,

    // presenters
    projects: ProjectMap,
    launchers: LauncherMap,
    configuration: RuntimeConfiguration,
    instances: InstanceMap,

    /// The sizes of the instance title bar (ADR 0019).
    instance_title_bar_metrics: InstanceTitleBarMetrics,
}

impl Aggregates {
    pub fn new(
        hierarchy: OrderedHierarchy<DesktopTarget>,
        configuration: RuntimeConfiguration,
        instance_title_bar_metrics: InstanceTitleBarMetrics,
    ) -> Self {
        Self {
            hierarchy,
            projects: Map::default(),

            launchers: Map::default(),
            configuration,
            instances: Map::default(),
            instance_title_bar_metrics,
        }
    }

    /// The Full Screen Mode `instance` currently presents in: an assistant's
    /// temporary mode, or its launcher's mode shared by its primary instances.
    /// Every caller passes an instance of the live topology, whose launcher and
    /// its configuration record are invariants — layout, camera, and hover reads
    /// all run after the transaction's topology changes are applied.
    pub(super) fn instance_full_screen_mode(&self, instance: InstanceId) -> FullScreenMode {
        if let Some(mode) = self
            .instances
            .get(&instance)
            .and_then(|presenter| presenter.full_screen_mode())
        {
            mode
        } else {
            let launcher = self.hierarchy.launcher_of_instance(instance);
            self.configuration[launcher].full_screen_mode
        }
    }
}

impl DesktopSystem {
    pub fn new(
        env: DesktopEnvironment,
        default_panel_size: SizePx,
        instance_title_bar_metrics: InstanceTitleBarMetrics,
        aggregate: RuntimeConfiguration,
    ) -> Result<Self> {
        // Architecture: This is a direct requirement from the desktop presenter. But where does our
        // root location actually come from, shouldn't it be provided by the caller.
        let (_, location) = identity_location().submit();

        let desktop_presenter = DesktopPresenter::new(location);
        let zoom_level_indicator = ZoomLevelIndicatorPresenter::new();

        let event_router = EventRouter::new();

        let layout_state = DesktopLayoutState::new();

        let system = Self {
            env,

            default_panel_size,
            // The window starts at the extent of an instance: its title bar and its panel (ADR 0019).
            window_state: WindowState::new(
                instance_extent(default_panel_size, instance_title_bar_metrics.height),
                false,
            ),

            event_router,
            camera: CameraPresentation::new(PixelCamera::default()),
            zoom_level: ZoomLevel::Focus,
            navigation_control: NavigationControl::default(),
            deferred_focus_launcher_measures: Default::default(),
            layout_state,

            zoom_level_indicator,
            desktop_presenter,
            aggregates: Aggregates::new(
                OrderedHierarchy::default(),
                aggregate,
                instance_title_bar_metrics,
            ),
        };

        Ok(system)
    }

    // Architecture: Is it really necessary to think in terms of transaction, if we update the
    // effects explicitly?
    //
    // Not a transaction yet: a change that fails partway leaves the effects
    // of the earlier changes applied, so the state may be inconsistent
    // (including the document mirror, which lands before the apply — see the
    // `DesktopChange::Project` arm in `apply_change`).
    /// Applies `changes` to completion and returns effects for the host to execute.
    /// Configuration persistence remains outside the system (ADR 0013); setup
    /// transactions emit no persistence effect because they replay parsed state.
    pub fn transact(
        &mut self,
        changes: impl Into<Changes>,
        instance_manager: &mut InstanceManager,
        effects_mode: impl Into<Option<TransactionEffectsMode>>,
    ) -> Result<TransactionOutput> {
        let changes = changes.into();
        let previous_zoom_level_state = self.zoom_level_state();
        let window_size = self.window_state.inner_size;
        // For live transactions the gesture mode is derived from the current pointer-button state;
        // callers only pass an explicit mode for setup.
        let effects_mode = effects_mode
            .into()
            .unwrap_or_else(|| self.live_effects_mode());

        // Run changes to completion and combine everything into a `ChangeSurface`.

        let mut change_surface = ChangeSurface::default();
        let mut system_effects = Vec::new();
        {
            let mut changes: VecDeque<DesktopChange> = changes.into_iter().collect();
            while let Some(change) = changes.pop_front() {
                let output = self.apply_change(change, instance_manager)?; // TODO: I think Changes should support a DoubleEndedIterator.
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

        if change_surface.window_fullscreen_changed {
            system_effects.push(DesktopSystemEffect::ToggleWindowFullScreen);
        }

        // A setup transaction replays configuration already on disk and never persists it (ADR 0013).
        if change_surface.configuration_changed && effects_mode != TransactionEffectsMode::Setup {
            system_effects.push(DesktopSystemEffect::PersistConfiguration);
        }

        // Collect deferred measures if the camera can be moved.

        // Detail: If camera moves are not allowed we assume that large visual changes aren't, too.
        // For example, focus layout effects.
        //
        // Design: may replace deferred_* with a ChangeSurface (a "deferred" ChangeSurface?).
        // Fullscreen transitions expose intermediate sizes; animating here makes the camera visibly leave the focused content.
        let camera_mode = if change_surface.window_size_changed
            && effects_mode == TransactionEffectsMode::Normal
        {
            CameraPresentationMode::Snap
        } else {
            effects_mode.camera_presentation_mode()
        };
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

        let zoom_level_changed = self.zoom_level_state() != previous_zoom_level_state;
        let update_zoom_level_indicator = zoom_level_changed || change_surface.window_size_changed;
        let update_camera = change_surface.camera_invalid();

        // Convert the change surface to effects.
        let effects = convert_change_surface_to_effects(change_surface);

        // Layout and camera focus for presenters that must fit into the window
        // read the size from the system's window state.
        self.run_effects_to_completion(effects_mode, effects, instance_manager)?;

        // Resolve camera intent after all effects were run, when all placements are final.
        if update_camera {
            let desired = self.resolve_desired_camera();
            self.camera.set_desired(desired);
        }

        self.camera.synchronize(camera_mode);

        if update_zoom_level_indicator {
            self.zoom_level_indicator.sync_layout(window_size);
        }
        if effects_mode != TransactionEffectsMode::Setup
            && zoom_level_changed
            && let Some((zoom_level, project_depth)) = self.zoom_navigation().focused_zoom_level()
        {
            self.zoom_level_indicator.show(zoom_level, project_depth);
        }

        // Update the hover target.
        self.desktop_presenter
            .set_hover_placement(self.hover_placement());

        Ok(TransactionOutput {
            effects: system_effects,
        })
    }

    fn zoom_level_state(&self) -> ZoomLevelState {
        ZoomLevelState {
            zoom_level: self.zoom_level,
            keyboard_focus: self.event_router.keyboard_focus().cloned(),
        }
    }

    pub fn is_present(&self, instance: &InstanceId) -> bool {
        self.aggregates.instances.contains_key(instance)
    }

    /// The live configuration aggregate, for the caller's persistence (ADR 0013).
    pub fn configuration(&self) -> &RuntimeConfiguration {
        &self.aggregates.configuration
    }

    pub fn camera(&mut self) -> &PixelCamera {
        self.camera.proceed()
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
        self.hierarchy
            .get_nested(&instance.into())
            .iter()
            .find_map(|target| match target {
                DesktopTarget::View(view) => Some(*view),
                _ => None,
            })
    }

    // The parent project always has a presenter before it can host a slot, so
    // a missing one is an invariant violation.
    pub fn project_matrix_location(&self, project: ProjectId) -> Handle<Location> {
        self.projects
            .get(&project)
            .unwrap_or_else(|| panic!("project {project:?} has no presenter"))
            .matrix
            .location()
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

#[cfg(test)]
mod tests {
    use super::ZoomLevel;

    #[test]
    fn zoom_steps_on_an_instance_pass_every_level() {
        assert_eq!(ZoomLevel::Focus.zoom_out(true), Some(ZoomLevel::Slot));
        assert_eq!(ZoomLevel::Slot.zoom_out(true), Some(ZoomLevel::Row));
        assert_eq!(ZoomLevel::Row.zoom_out(true), Some(ZoomLevel::Project));
        assert_eq!(ZoomLevel::Project.zoom_out(true), None);

        assert_eq!(ZoomLevel::Project.zoom_in(true), Some(ZoomLevel::Row));
        assert_eq!(ZoomLevel::Row.zoom_in(true), Some(ZoomLevel::Slot));
        assert_eq!(ZoomLevel::Slot.zoom_in(true), Some(ZoomLevel::Focus));
        assert_eq!(ZoomLevel::Focus.zoom_in(true), None);
    }

    #[test]
    fn zoom_steps_on_other_targets_merge_slot_into_focus() {
        assert_eq!(ZoomLevel::Slot.normalized(false), ZoomLevel::Focus);
        assert_eq!(ZoomLevel::Focus.zoom_out(false), Some(ZoomLevel::Row));
        assert_eq!(ZoomLevel::Slot.zoom_out(false), Some(ZoomLevel::Row));
        assert_eq!(ZoomLevel::Row.zoom_in(false), Some(ZoomLevel::Focus));
        assert_eq!(ZoomLevel::Slot.zoom_in(false), None);
    }
}
