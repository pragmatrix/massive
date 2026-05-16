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

use anyhow::Result;
use derive_more::Debug;
use std::collections::HashSet;
use std::time::Duration;

use massive_animation::Animated;
use massive_applications::{InstanceId, ViewId};
use massive_geometry::{PixelCamera, SizePx};
use massive_layout::{LayoutTopology, Placement};
use massive_scene::{Location, Object, Transform};
use massive_shell::{FontManager, Scene};

pub use commands::{DesktopCommand, ProjectCommand};
pub use effects::Effects;
use layout_algorithm::DesktopLayoutAlgorithm;
pub(crate) use layout_algorithm::place_container_children;
use layout_state::DesktopLayoutState;

use crate::event_sourcing::{self, Transaction};
use crate::focus_path::FocusPath;
use crate::focus_path::PathResolver;
use crate::instance_manager::InstanceManager;
use crate::instance_presenter::InstancePresenter;
use crate::projects::{
    DesktopPresenter, LaunchProfileId, LauncherPresenter, ProjectId, ProjectPresenter,
};
use crate::{DesktopEnvironment, EventRouter, Map, OrderedHierarchy};

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

pub type Cmd = event_sourcing::Cmd<DesktopCommand>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransactionEffectsMode {
    #[default]
    Normal,
    Setup,
    CameraLocked,
}

impl TransactionEffectsMode {
    pub fn animate(self) -> bool {
        match self {
            TransactionEffectsMode::Setup => false,
            TransactionEffectsMode::CameraLocked => true,
            TransactionEffectsMode::Normal => true,
        }
    }

    pub fn permit_camera_moves(self) -> bool {
        match self {
            TransactionEffectsMode::Setup => true,
            TransactionEffectsMode::CameraLocked => false,
            TransactionEffectsMode::Normal => true,
        }
    }
}

#[derive(Debug)]
pub struct DesktopSystem {
    env: DesktopEnvironment,
    fonts: FontManager,

    default_panel_size: SizePx,

    event_router: EventRouter<DesktopTarget>,
    camera: Animated<PixelCamera>,
    pointer_feedback_enabled: bool,
    deferred_focus_layout_launchers: HashSet<LaunchProfileId>,

    #[debug(skip)]
    layout_state: DesktopLayoutState,

    aggregates: Aggregates,
}

/// Aggregates are separated, so that we can control borrowing them in a more granular way.
#[derive(Debug)]
struct Aggregates {
    hierarchy: OrderedHierarchy<DesktopTarget>,

    startup_profile: Option<LaunchProfileId>,

    // presenters
    desktop_presenter: DesktopPresenter,
    projects: Map<ProjectId, ProjectPresenter>,
    launchers: Map<LaunchProfileId, LauncherPresenter>,
    instances: Map<InstanceId, InstancePresenter>,
}

impl Aggregates {
    pub fn new(
        hierarchy: OrderedHierarchy<DesktopTarget>,
        desktop_presenter: DesktopPresenter,
    ) -> Self {
        Self {
            hierarchy,
            startup_profile: None,
            projects: Map::default(),

            desktop_presenter,
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

        let desktop_presenter = DesktopPresenter::new(location, scene);

        let event_router = EventRouter::new();

        let layout_state = DesktopLayoutState::new();

        let system = Self {
            env,
            fonts,

            default_panel_size,

            event_router,
            camera: scene.animated(PixelCamera::default()),
            pointer_feedback_enabled: true,
            deferred_focus_layout_launchers: HashSet::new(),
            layout_state,

            aggregates: Aggregates::new(OrderedHierarchy::default(), desktop_presenter),
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
        effects_mode: impl Into<Option<TransactionEffectsMode>>,
    ) -> Result<()> {
        let effects_mode = effects_mode.into().unwrap_or_default();

        self.transact_with_effects(
            transaction,
            scene,
            instance_manager,
            effects_mode,
            Effects::None,
        )
    }

    pub fn transact_with_effects(
        &mut self,
        transaction: impl Into<Transaction<DesktopCommand>>,
        scene: &Scene,
        instance_manager: &mut InstanceManager,
        effects_mode: TransactionEffectsMode,
        initial_effects: Effects,
    ) -> Result<()> {
        let mut command_effects = initial_effects;

        for command in transaction.into() {
            command_effects += self.apply_command(command, scene, instance_manager)?;
        }

        self.run_effects_to_completion(effects_mode, self.transaction_effects(command_effects))?;

        Ok(())
    }

    pub fn apply_animations(&mut self) {
        let focused_instance = self
            .aggregates
            .hierarchy
            .resolve_path(self.event_router.focused())
            .instance();

        let launcher_instance_ids: Vec<_> = self
            .aggregates
            .launchers
            .keys()
            .copied()
            .map(|launcher_id| {
                (
                    launcher_id,
                    self.aggregates.launcher_instance_ids(launcher_id),
                )
            })
            .collect();

        for (launcher_id, child_instances) in launcher_instance_ids {
            self.aggregates
                .launchers
                .get_mut(&launcher_id)
                .expect("Launcher missing")
                .apply_animations(
                    &mut self.aggregates.instances,
                    &child_instances,
                    focused_instance,
                );
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

    pub fn camera(&self) -> PixelCamera {
        self.camera.value()
    }

    pub fn is_cursor_visible(&self) -> bool {
        self.pointer_feedback_enabled
    }

    pub fn any_buttons_pressed(&self) -> bool {
        self.event_router.any_buttons_pressed()
    }

    /// Remove the target from the hierarchy. Specific target aggregates are left
    /// untouched (they may be needed for fading out, etc.).
    fn remove_target(&mut self, target: &DesktopTarget) -> Result<Effects> {
        // Check if all components that hold reference actually removed them.
        self.event_router.notify_removed(target)?;

        let parent = self
            .aggregates
            .hierarchy
            .parent(target)
            .cloned()
            .expect("Internal error: remove_target called for root target");

        // Explicitly invalidate layout cache for the removed subtree.
        self.layout_state
            .remove_subtree(target, &self.aggregates.hierarchy);

        // Finally remove them.
        self.aggregates.hierarchy.remove(target)?;
        // Mark the surviving parent, not the removed node:
        // - removed nodes are ignored by incremental recompute root collection,
        // - parent refresh updates cached children and recomputes sibling placement.
        Ok(effects::DesktopEffect::Measure(parent).into())
    }

    fn placement(&self, target: &DesktopTarget) -> Option<Placement<Transform, 2>> {
        self.layout_state
            .absolute_placement(target, &self.aggregates.hierarchy)
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

    pub fn launcher_instance_ids(&self, launcher_id: LaunchProfileId) -> Vec<InstanceId> {
        self.hierarchy
            .get_nested(&DesktopTarget::Launcher(launcher_id))
            .iter()
            .map(|target| match target {
                DesktopTarget::Instance(instance_id) => *instance_id,
                _ => panic!("launcher children must be instances"),
            })
            .collect()
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

    fn parent_of(&self, id: &DesktopTarget) -> Option<DesktopTarget> {
        self.parent(id).cloned()
    }
}
