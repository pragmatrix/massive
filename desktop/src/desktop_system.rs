//! The Desktop as an event sourced user interface system.
//!
//! The presenter hierarchy is treated as an aggregate built up from the events.
//!
//! The decision to use event sourcing stems from the fact that we want to run everything as
//! incrementally as possible, because we want to add projects, etc.
//!
//! The goal here is to remove as much as possible from the specific instances into separate systems
//! and aggregates that are event driven.

use std::cmp::max;

use anyhow::{Result, anyhow, bail};
use derive_more::{Debug, From};
use log::warn;
use winit::event::ElementState;
use winit::keyboard::{Key, NamedKey};

use massive_animation::{Animated, Interpolation};
use massive_applications::{
    CreationMode, InstanceId, InstanceParameters, ViewCreationInfo, ViewEvent, ViewId, ViewRole,
};
use massive_geometry::{PixelCamera, PointPx, Rect, RectPx, SizePx};
use massive_input::Event;
use massive_layout::{
    IncrementalLayouter, LayoutAlgorithm, LayoutAxis, LayoutTopology, Offset, Rect as LayoutRect,
    Size,
};
use massive_renderer::RenderGeometry;
use massive_scene::{Location, Object, ToCamera, Transform};
use massive_shell::{FontManager, Scene};

use crate::event_router::EventTransitions;
use crate::event_sourcing::{self, Transaction};
use crate::focus_path::{FocusPath, PathResolver};
use crate::hit_tester::AggregateHitTester;
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::instance_presenter::{
    InstancePresenter, InstancePresenterState, PrimaryViewPresenter, STRUCTURAL_ANIMATION_DURATION,
};
use crate::layout::{LayoutSpec, ToContainer};
use crate::navigation::ordered_rects_in_direction;
use crate::projects::{
    GroupId, GroupPresenter, LaunchGroupProperties, LaunchProfile, LaunchProfileId, LauncherMode,
    LauncherPresenter, ProjectPresenter,
};
use crate::send_transition::{SendTransition, convert_to_send_transitions};
use crate::{DesktopEnvironment, DirectionBias, EventRouter, Map, OrderedHierarchy, navigation};

const SECTION_SPACING: u32 = 20;
const VISOR_INSTANCE_FORWARD_Z: f64 = 128.0;

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

/// The commands the desktop system can execute.
#[derive(Debug)]
pub enum DesktopCommand {
    Project(ProjectCommand),
    StartInstance {
        launcher: LaunchProfileId,
        parameters: InstanceParameters,
    },
    StopInstance(InstanceId),
    PresentInstance {
        launcher: LaunchProfileId,
        instance: InstanceId,
    },
    PresentView(InstanceId, ViewCreationInfo),
    HideView(ViewPath),
    ZoomOut,
    Navigate(navigation::Direction),
}

#[derive(Debug)]
pub enum ProjectCommand {
    // Project Configuration
    AddLaunchGroup {
        parent: Option<GroupId>,
        id: GroupId,
        properties: LaunchGroupProperties,
    },
    #[allow(unused)]
    RemoveLaunchGroup(GroupId),
    AddLauncher {
        group: GroupId,
        id: LaunchProfileId,
        profile: LaunchProfile,
    },
    #[allow(unused)]
    RemoveLauncher(LaunchProfileId),
    SetStartupProfile(Option<LaunchProfileId>),
}

pub type Cmd = event_sourcing::Cmd<DesktopCommand>;

#[derive(Debug)]
pub struct DesktopSystem {
    env: DesktopEnvironment,
    fonts: FontManager,

    default_panel_size: SizePx,

    event_router: EventRouter<DesktopTarget>,
    camera: Animated<PixelCamera>,

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

                let target = instance.into();
                let focused_path = self
                    .aggregates
                    .hierarchy
                    .resolve_path(self.event_router.focused());

                let was_focused = focused_path.instance() == Some(instance);
                let focus_neighbor = if was_focused {
                    self.aggregates
                        .hierarchy
                        .entry(&target)
                        .neighbor(DirectionBias::Begin)
                        .cloned()
                } else {
                    None
                };

                self.unfocus(instance.into(), instance_manager)?;

                // Robustness: May add neighbor selection to unfocus as an option?
                if let Some(neighbor_instance) = focus_neighbor {
                    if let [DesktopTarget::View(view)] =
                        self.aggregates.hierarchy.get_nested(&neighbor_instance)
                    {
                        assert!(
                            self.focus(&DesktopTarget::View(*view), instance_manager)?
                                .is_none()
                        )
                    } else {
                        assert!(self.focus(&neighbor_instance, instance_manager)?.is_none())
                    }
                }

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

    /// Update all effects.
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

    pub fn process_input_event(
        &mut self,
        event: &Event<ViewEvent>,
        instance_manager: &InstanceManager,
        render_geometry: &RenderGeometry,
    ) -> Result<Cmd> {
        let keyboard_cmd = self.preprocess_keyboard_input(event)?;
        if !keyboard_cmd.is_none() {
            return Ok(keyboard_cmd);
        }

        let hit_tester = AggregateHitTester::new(
            &self.aggregates.hierarchy,
            &self.layouter,
            &self.aggregates.launchers,
            &self.aggregates.instances,
            render_geometry,
        );

        let transitions = self.event_router.process(event, &hit_tester)?;

        self.forward_event_transitions(transitions, instance_manager)
    }

    fn focus(&mut self, target: &DesktopTarget, instance_manager: &InstanceManager) -> Result<Cmd> {
        let transitions = self.event_router.focus(target);
        self.forward_event_transitions(transitions, instance_manager)
    }

    /// If the target is involved in any focus path, unfocus it.
    ///
    /// For the keyboard focus, this focuses the parent.
    ///
    /// For the cursor focus, this clears the focus (we can't refocus here using the hit tester,
    /// because the target may be in the hierarchy).
    fn unfocus(&mut self, target: DesktopTarget, instance_manager: &InstanceManager) -> Result<()> {
        // Keyboard focus

        let focus = self.event_router.focused();
        let focus_path = self.aggregates.hierarchy.resolve_path(focus);
        // Optimization: The parent can be resolved directly from the focus path.
        if focus_path.contains(&target)
            && let Some(parent) = self.aggregates.hierarchy.parent(&target)
        {
            assert!(self.focus(&parent.clone(), instance_manager)?.is_none());
        }

        let pointer_focus = self.event_router.pointer_focus();
        let focus_path = self.aggregates.hierarchy.resolve_path(pointer_focus);
        if focus_path.contains(&target) {
            let transitions = self.event_router.unfocus_pointer()?;
            assert!(
                self.forward_event_transitions(transitions, instance_manager)?
                    .is_none()
            );
        }
        Ok(())
    }

    #[allow(unused)]
    fn refocus_pointer(
        &mut self,
        instance_manager: &InstanceManager,
        render_geometry: &RenderGeometry,
    ) -> Result<Cmd> {
        let transitions = self
            .event_router
            .reset_pointer_focus(&AggregateHitTester::new(
                &self.aggregates.hierarchy,
                &self.layouter,
                &self.aggregates.launchers,
                &self.aggregates.instances,
                render_geometry,
            ))?;

        self.forward_event_transitions(transitions, instance_manager)
    }

    fn present_instance(
        &mut self,
        launcher: LaunchProfileId,
        originating_from: Option<InstanceId>,
        instance: InstanceId,
        scene: &Scene,
    ) -> Result<usize> {
        let originating_presenter = originating_from
            .and_then(|originating_from| self.aggregates.instances.get(&originating_from));

        let background_for_instance = self
            .aggregates
            .launchers
            .get(&launcher)
            .expect("Launcher not found")
            .mode()
            == LauncherMode::Visor;

        // Correctness: We animate from 0,0 if no originating exist. Need a position here.
        let initial_center_translation = originating_presenter
            .map(|op| op.center_translation_animation.value())
            .unwrap_or_default();

        let presenter = InstancePresenter::new(
            initial_center_translation,
            background_for_instance,
            self.aggregates.project_presenter.location.clone(),
            scene,
        );

        self.aggregates.instances.insert(instance, presenter)?;

        let nested = self.aggregates.hierarchy.get_nested(&launcher.into());
        let insertion_pos = if let Some(originating_from) = originating_from {
            nested
                .iter()
                .position(|i| *i == DesktopTarget::Instance(originating_from))
                .map(|i| i + 1)
                .unwrap_or(nested.len())
        } else {
            0
        };

        // Inform the launcher to fade out.
        self.aggregates
            .launchers
            .get_mut(&launcher)
            .expect("Launcher not found")
            .fade_out();

        Ok(insertion_pos)
    }

    fn hide_instance(&mut self, instance: InstanceId) -> Result<()> {
        let Some(DesktopTarget::Launcher(launcher)) =
            self.aggregates.hierarchy.parent(&instance.into()).cloned()
        else {
            bail!("Internal error: Launcher not found");
        };

        self.remove_target(&DesktopTarget::Instance(instance))?;
        self.aggregates.instances.remove(&instance)?;

        if !self
            .aggregates
            .hierarchy
            .entry(&launcher.into())
            .has_nested()
        {
            self.aggregates
                .launchers
                .get_mut(&launcher)
                .expect("Launcher not found")
                .fade_in();
        }

        Ok(())
    }

    fn present_view(
        &mut self,
        instance: InstanceId,
        view_creation_info: &ViewCreationInfo,
    ) -> Result<()> {
        if view_creation_info.role != ViewRole::Primary {
            todo!("Only primary views are supported yet");
        }

        let Some(instance_presenter) = self.aggregates.instances.get_mut(&instance) else {
            bail!("Instance not found");
        };

        if !matches!(
            instance_presenter.state,
            InstancePresenterState::WaitingForPrimaryView
        ) {
            bail!("Primary view is already presenting");
        }

        // Architecture: Move this transition in the InstancePresenter
        //
        // Feature: Add a alpha animation just for the view.
        instance_presenter.state = InstancePresenterState::Presenting {
            view: PrimaryViewPresenter {
                creation_info: view_creation_info.clone(),
            },
        };

        // Add the view to the hierarchy.
        self.aggregates.hierarchy.add(
            DesktopTarget::Instance(instance),
            DesktopTarget::View(view_creation_info.id),
        )?;
        self.layouter
            .mark_reflow_pending(DesktopTarget::Instance(instance));

        Ok(())
    }

    fn hide_view(&mut self, path: ViewPath) -> Result<()> {
        let Some(instance_presenter) = self.aggregates.instances.get_mut(&path.instance) else {
            warn!("Can't hide view: Instance for view not found");
            // Robustness: Decide if this should return an error.
            return Ok(());
        };

        // Architecture: Move this into the InstancePresenter (don't make state pub).
        match &instance_presenter.state {
            InstancePresenterState::WaitingForPrimaryView => {
                bail!(
                    "A view needs to be hidden, but instance presenter waits for a view with a primary role."
                )
            }
            InstancePresenterState::Presenting { view } => {
                if view.creation_info.id == path.view {
                    // Feature: this should initiate a disappearing animation?
                    instance_presenter.state = InstancePresenterState::Disappearing;
                } else {
                    bail!("Invalid view: It's not related to anything we present");
                }
            }
            InstancePresenterState::Disappearing => {
                // ignored, we are already disappearing.
            }
        }

        // Robustness: What about focus?

        // And remove the view.
        self.remove_target(&DesktopTarget::View(path.view))?;

        Ok(())
    }

    fn apply_layout_changes(
        &mut self,
        changed: Vec<(DesktopTarget, LayoutRect<2>)>,
        animate: bool,
    ) {
        for (id, layout_rect) in changed {
            let rect_px: RectPx = layout_rect.into();
            let rect: Rect = rect_px.into();

            match id {
                DesktopTarget::Desktop => {}
                DesktopTarget::Instance(instance_id) => {
                    let z_offset = self.instance_layout_depth(instance_id);
                    self.aggregates
                        .instances
                        .get_mut(&instance_id)
                        .expect("Instance missing")
                        .set_rect(rect_px, z_offset, animate);
                }
                DesktopTarget::Group(group_id) => {
                    self.aggregates
                        .groups
                        .get_mut(&group_id)
                        .expect("Missing group")
                        .rect = rect;
                }
                DesktopTarget::Launcher(launcher_id) => {
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
    }

    fn preprocess_keyboard_input(&self, event: &Event<ViewEvent>) -> Result<Cmd> {
        // Catch CMD+t and CMD+w if an instance has the keyboard focus.

        if let ViewEvent::KeyboardInput {
            event: key_event, ..
        } = event.event()
            && key_event.state == ElementState::Pressed
            && !key_event.repeat
            && event.device_states().is_command()
        {
            let focused = self.event_router.focused();
            let focused = self.aggregates.hierarchy.resolve_path(focused);

            // Simplify: Instance should probably return the launcher, too now.
            if let Some(instance) = focused.instance()
                && let Some(DesktopTarget::Launcher(launcher)) =
                    self.aggregates.hierarchy.parent(&instance.into())
            {
                match &key_event.logical_key {
                    Key::Character(c) if c.as_str() == "t" => {
                        return Ok(DesktopCommand::StartInstance {
                            launcher: *launcher,
                            parameters: Default::default(),
                        }
                        .into());
                    }
                    Key::Character(c) if c.as_str() == "w" => {
                        // Architecture: Shouldn't this just end the current view, and let the
                        // instance decide then?
                        return Ok(DesktopCommand::StopInstance(instance).into());
                    }
                    _ => {}
                }
            }

            if let Some(direction) = match &key_event.logical_key {
                Key::Named(NamedKey::ArrowLeft) => Some(navigation::Direction::Left),
                Key::Named(NamedKey::ArrowRight) => Some(navigation::Direction::Right),
                Key::Named(NamedKey::ArrowUp) => Some(navigation::Direction::Up),
                Key::Named(NamedKey::ArrowDown) => Some(navigation::Direction::Down),
                _ => None,
            } {
                return Ok(DesktopCommand::Navigate(direction).into());
            }

            if let Key::Named(NamedKey::Escape) = &key_event.logical_key {
                return Ok(DesktopCommand::ZoomOut.into());
            }
        }

        Ok(Cmd::None)
    }

    fn forward_event_transitions(
        &mut self,
        transitions: EventTransitions<DesktopTarget>,
        instance_manager: &InstanceManager,
    ) -> Result<Cmd> {
        let mut cmd = Cmd::None;

        let keyboard_modifiers = self.event_router.keyboard_modifiers();

        let send_transitions = convert_to_send_transitions(
            transitions,
            keyboard_modifiers,
            &self.aggregates.hierarchy,
        );

        // Robustness: While we need to forward all transitions we currently process only one intent.
        for transition in send_transitions {
            cmd += self.forward_event_transition(transition, instance_manager)?;
        }

        Ok(cmd)
    }

    /// Forward event transitions to the appropriate handler based on the target type.
    fn forward_event_transition(
        &mut self,
        SendTransition(target, event): SendTransition<DesktopTarget>,
        instance_manager: &InstanceManager,
    ) -> Result<Cmd> {
        // Route to the appropriate handler based on the last target in the path
        match target {
            DesktopTarget::Desktop => {}
            DesktopTarget::Instance(..) => {}
            DesktopTarget::View(view_id) => {
                let path = self
                    .aggregates
                    .hierarchy
                    .resolve_path(Some(&view_id.into()));
                let Some(instance) = path.instance() else {
                    // This happens when the instance is gone (resolve_path returns only the view, because it puts it by default in the first position).
                    warn!(
                        "Instance of view {view_id:?} not found (path: {path:?}), can't deliver event: {event:?}"
                    );
                    return Ok(Cmd::None);
                };

                // Need to translate the event. The view has its own coordinate system.
                let event = if let Some(rect) = self.rect(&target) {
                    event.translate(-rect.origin())
                } else {
                    // This happens on startup on PresentView, because the layout isn't there yet.
                    event
                };

                if let Err(e) = instance_manager.send_view_event((instance, view_id), event.clone())
                {
                    // This might happen when an instance ends, but we haven't yet received the
                    // information.
                    warn!("Sending view event {event:?} failed with {e}");
                }
            }
            DesktopTarget::Group(..) => {}
            DesktopTarget::Launcher(launcher_id) => {
                // Architecture: Shouldn't we move the hovering into the launcher presenters or even into the system?
                match event {
                    ViewEvent::CursorEntered => {
                        let launcher = &self.aggregates.launchers[&launcher_id];
                        let rect = launcher.rect.final_value();
                        self.aggregates.project_presenter.show_hover_rect(rect);
                    }
                    ViewEvent::CursorLeft => {
                        self.aggregates.project_presenter.hide_hover_rect();
                    }
                    event => {
                        let launcher = self
                            .aggregates
                            .launchers
                            .get_mut(&launcher_id)
                            .expect("Launcher not found");
                        return launcher.process(event);
                    }
                }
            }
        }

        Ok(Cmd::None)
    }

    // Camera

    pub fn camera_for_focus(&self, focus: &DesktopTarget) -> Option<PixelCamera> {
        match focus {
            // Desktop and TopBand are constrained to their size.
            DesktopTarget::Desktop => self
                .rect(&DesktopTarget::Desktop)
                .map(|rect| rect.to_camera()),

            DesktopTarget::Group(group) => {
                Some(self.aggregates.groups[group].rect.center().to_camera())
            }
            DesktopTarget::Launcher(launcher) => Some(
                self.aggregates.launchers[launcher]
                    .rect
                    .final_value()
                    .center()
                    .to_camera(),
            ),

            DesktopTarget::Instance(instance_id) => {
                let instance = &self.aggregates.instances[instance_id];
                let transform: Transform =
                    instance.center_translation_animation.final_value().into();
                Some(transform.to_camera())
            }
            DesktopTarget::View(_) => {
                // Forward this to the parent (which must be a ::Instance).
                self.camera_for_focus(self.aggregates.hierarchy.parent(focus)?)
            }
        }
    }

    fn locate_navigation_candidate(
        &self,
        from: &DesktopTarget,
        direction: navigation::Direction,
    ) -> Option<DesktopTarget> {
        if !matches!(
            from,
            DesktopTarget::Launcher(..) | DesktopTarget::Instance(..) | DesktopTarget::View(..),
        ) {
            return None;
        }

        let from_rect = self.rect(from)?;
        let launcher_targets_without_instances = self
            .aggregates
            .launchers
            .keys()
            .map(|l| DesktopTarget::Launcher(*l))
            .filter(|t| self.aggregates.hierarchy.get_nested(t).is_empty());
        let all_instances_or_views = self.aggregates.instances.keys().map(|instance| {
            if let Some(view) = self.aggregates.view_of_instance(*instance) {
                DesktopTarget::View(view)
            } else {
                DesktopTarget::Instance(*instance)
            }
        });
        let navigation_candidates = launcher_targets_without_instances
            .chain(all_instances_or_views)
            .filter_map(|target| self.rect(&target).map(|rect| (target, rect)));

        let ordered =
            ordered_rects_in_direction(from_rect.center(), direction, navigation_candidates);
        if let Some((nearest, _rect)) = ordered.first() {
            return Some(nearest.clone());
        }
        None
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

impl DesktopSystem {
    fn instance_layout_depth(&self, instance_id: InstanceId) -> f64 {
        let instance_target = DesktopTarget::Instance(instance_id);
        let is_visor_with_multiple_instances =
            match self.aggregates.hierarchy.parent(&instance_target) {
                Some(DesktopTarget::Launcher(launcher_id)) => {
                    self.aggregates.launchers[launcher_id].mode() == LauncherMode::Visor
                        && self.aggregates.hierarchy.group(&instance_target).len() > 1
                }
                _ => false,
            };

        if is_visor_with_multiple_instances {
            VISOR_INSTANCE_FORWARD_Z
        } else {
            0.0
        }
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

struct DesktopLayoutAlgorithm<'a> {
    aggregates: &'a Aggregates,
    default_panel_size: SizePx,
}

impl DesktopLayoutAlgorithm<'_> {
    fn resolve_layout_spec(&self, target: &DesktopTarget) -> LayoutSpec {
        match target {
            DesktopTarget::Desktop => LayoutAxis::VERTICAL
                .to_container()
                .spacing(SECTION_SPACING)
                .into(),
            DesktopTarget::Group(group_id) => self.aggregates.groups[group_id]
                .properties
                .layout
                .axis()
                .to_container()
                .spacing(10)
                .padding((10, 10))
                .into(),
            DesktopTarget::Launcher(_) => {
                if self.aggregates.hierarchy.get_nested(target).is_empty() {
                    self.default_panel_size.into()
                } else {
                    LayoutAxis::HORIZONTAL.into()
                }
            }
            DesktopTarget::Instance(instance) => {
                let instance = &self.aggregates.instances[instance];
                if !instance.presents_primary_view() {
                    self.default_panel_size.into()
                } else {
                    LayoutAxis::HORIZONTAL.into()
                }
            }
            DesktopTarget::View(_) => self.default_panel_size.into(),
        }
    }

    fn place_container_children(
        axis: LayoutAxis,
        spacing: i32,
        mut offset: Offset<2>,
        child_sizes: &[Size<2>],
    ) -> Vec<Offset<2>> {
        let axis_index: usize = axis.into();
        let mut child_offsets = Vec::with_capacity(child_sizes.len());

        for (index, &child_size) in child_sizes.iter().enumerate() {
            if index > 0 {
                offset[axis_index] += spacing;
            }
            child_offsets.push(offset);
            offset[axis_index] += child_size[axis_index] as i32;
        }

        child_offsets
    }
}

impl LayoutAlgorithm<DesktopTarget, 2> for DesktopLayoutAlgorithm<'_> {
    fn measure(&self, id: &DesktopTarget, child_sizes: &[Size<2>]) -> Size<2> {
        if let DesktopTarget::Launcher(launcher_id) = id
            && self.aggregates.launchers[launcher_id].mode() == LauncherMode::Visor
        {
            return self.default_panel_size.into();
        }

        match self.resolve_layout_spec(id) {
            LayoutSpec::Leaf(size) => size.into(),
            LayoutSpec::Container {
                axis,
                padding,
                spacing,
            } => {
                let axis = *axis;
                let mut inner_size = Size::EMPTY;

                for (index, &child_size) in child_sizes.iter().enumerate() {
                    for dim in 0..2 {
                        if dim == axis {
                            inner_size[dim] += child_size[dim];
                            if index > 0 {
                                inner_size[dim] += spacing;
                            }
                        } else {
                            inner_size[dim] = max(inner_size[dim], child_size[dim]);
                        }
                    }
                }

                padding.leading + inner_size + padding.trailing
            }
        }
    }

    fn place_children(
        &self,
        id: &DesktopTarget,
        parent_offset: Offset<2>,
        child_sizes: &[Size<2>],
    ) -> Vec<Offset<2>> {
        if let DesktopTarget::Launcher(launcher_id) = id
            && self.aggregates.launchers[launcher_id].mode() == LauncherMode::Visor
        {
            let axis = LayoutAxis::HORIZONTAL;
            let axis_index: usize = axis.into();
            let spacing = 0i32;

            let children_span: i32 = child_sizes
                .iter()
                .map(|size| size[axis_index] as i32)
                .sum::<i32>()
                + spacing * (child_sizes.len().saturating_sub(1) as i32);
            let panel_span = self.default_panel_size.width as i32;
            let center_offset = (panel_span - children_span) / 2;

            let mut offset = parent_offset;
            offset[axis_index] += center_offset;

            return Self::place_container_children(axis, spacing, offset, child_sizes);
        }

        match self.resolve_layout_spec(id) {
            LayoutSpec::Leaf(_) => Vec::new(),
            LayoutSpec::Container {
                axis,
                padding,
                spacing,
            } => {
                let offset = parent_offset + Offset::from(padding.leading);

                Self::place_container_children(axis, spacing as i32, offset, child_sizes)
            }
        }
    }
}

// Path utilities

impl DesktopFocusPath {
    pub fn instance(&self) -> Option<InstanceId> {
        self.iter().rev().find_map(|t| match t {
            DesktopTarget::Instance(id) => Some(*id),
            _ => None,
        })
    }

    /// Is this or a parent something that can be added new instances to?
    pub fn instance_parent(&self) -> Option<DesktopFocusPath> {
        self.iter()
            .enumerate()
            .rev()
            .find_map(|(i, t)| match t {
                DesktopTarget::Desktop => None,
                DesktopTarget::Group(..) => None,
                DesktopTarget::Launcher(..) => Some(i + 1),
                DesktopTarget::Instance(..) => Some(i),
                DesktopTarget::View(..) => {
                    assert!(matches!(self[i - 1], DesktopTarget::Instance(..)));
                    Some(i - 1)
                }
            })
            .map(|i| self.iter().take(i).cloned().collect::<Vec<_>>().into())
    }
}
