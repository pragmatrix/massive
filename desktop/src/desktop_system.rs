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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionEffectsMode {
    Setup,
    CameraLocked,
}

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
        effects_mode: impl Into<Option<TransactionEffectsMode>>,
    ) -> Result<()> {
        let effects_mode = effects_mode.into();

        for command in transaction.into() {
            self.apply_command(command, scene, instance_manager)?;
        }

        self.update_effects(effects_mode)?;

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

    pub fn is_cursor_visible(&self) -> bool {
        self.pointer_feedback_enabled
    }

    /// Remove the target from the hierarchy. Specific target aggregates are left
    /// untouched (they may be needed for fading out, etc.).
    fn remove_target(&mut self, target: &DesktopTarget) -> Result<()> {
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
