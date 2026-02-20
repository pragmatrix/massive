//! The Desktop as an event sourced user interface system.
//!
//! The presenter hierarchy is treated as an aggregate built up from the events.
//!
//! The decision to use event sourcing stems from the fact that we want to run everything as
//! incrementally as possible, because we want to add projects, etc.
//!
//! The goal here is to remove as much as possible from the specific instances into separate systems
//! and aggregates that are event driven.

use std::collections::HashMap;

use anyhow::{Result, anyhow, bail};
use derive_more::From;
use log::warn;
use winit::event::ElementState;
use winit::keyboard::{Key, NamedKey};

use massive_animation::{Animated, Interpolation};
use massive_applications::{
    CreationMode, InstanceId, InstanceParameters, ViewCreationInfo, ViewEvent, ViewId, ViewRole,
};
use massive_geometry::{
    Contains, Matrix4, PixelCamera, Point, PointPx, Rect, RectPx, SizePx, Vector3,
};
use massive_input::Event;
use massive_layout::{Layout, LayoutAxis, container, leaf};
use massive_renderer::RenderGeometry;
use massive_scene::{Location, Object, ToCamera, Transform};
use massive_shell::{FontManager, Scene};

use crate::event_router::EventTransitions;
use crate::event_sourcing::{self, Transaction};
use crate::focus_path::{FocusPath, PathResolver};
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::instance_presenter::{
    InstancePresenter, InstancePresenterState, PrimaryViewPresenter, STRUCTURAL_ANIMATION_DURATION,
};
use crate::layout::{LayoutSpec, ToContainer};
use crate::navigation::ordered_rects_in_direction;
use crate::projects::{
    GroupId, GroupPresenter, LaunchGroupProperties, LaunchProfile, LaunchProfileId,
    LauncherPresenter, ProjectPresenter,
};
use crate::send_transition::{SendTransition, convert_to_send_transitions};
use crate::{
    DesktopEnvironment, DirectionBias, EventRouter, HitTester, Map, OrderedHierarchy, navigation,
};

const SECTION_SPACING: u32 = 20;

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

    aggregates: Aggregates,
}

/// Aggregates are separated, so that we can control borrowing them in a more granular way.
#[derive(Debug)]
struct Aggregates {
    hierarchy: OrderedHierarchy<DesktopTarget>,
    rects: HashMap<DesktopTarget, Rect>,

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
            rects: HashMap::default(),
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

        let system = Self {
            env,
            fonts,

            default_panel_size,

            event_router,
            camera: scene.animated(PixelCamera::default()),

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
                self.aggregates.hierarchy.add(parent, id.into())?;
                self.aggregates
                    .groups
                    .insert(id, GroupPresenter::new(properties))?;
            }
            ProjectCommand::RemoveLaunchGroup(group) => {
                self.aggregates.remove_target(&group.into())?;
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
            }
            ProjectCommand::RemoveLauncher(id) => {
                let target = DesktopTarget::Launcher(id);
                self.aggregates.remove_target(&target)?;

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

        let layout = self.desktop_layout();
        self.apply_layout(layout, animate);

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

    // Architecture: We should probably not go through the old layout engine and think of something
    // more incremental.
    fn desktop_layout(&self) -> Layout<DesktopTarget, 2> {
        self.build_layout_for(DesktopTarget::Desktop)
    }

    fn build_layout_for(&self, target: DesktopTarget) -> Layout<DesktopTarget, 2> {
        match self.resolve_layout_spec(&target) {
            LayoutSpec::Container {
                axis,
                padding,
                spacing,
            } => {
                let mut container = container(target.clone(), axis)
                    .padding(padding)
                    .spacing(spacing);

                for nested in self.aggregates.hierarchy.get_nested(&target) {
                    container.nested(self.build_layout_for(nested.clone()));
                }

                container.layout()
            }
            LayoutSpec::Leaf(size) => leaf(target, size),
        }
    }

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
                // A launcher depends on the nested ones, if any, it's a horizontal, if none, its a
                // absolute size.
                // Architecture: A min size would make the nested check obsolete.
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
                    // We need to put the View below it
                    //
                    // Architecture: This feels wrong somehow, this mixes the focus hierarchy with
                    // the layout hierarchy. Do we need to separate them?
                    LayoutAxis::HORIZONTAL.into()
                }
            }
            DesktopTarget::View(_) =>
            // Assuming this is a primary view (for now).
            {
                self.default_panel_size.into()
            }
        }
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

        let hit_tester = &self.aggregates.hit_tester(render_geometry);

        let transitions = self.event_router.process(event, hit_tester)?;

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
            .reset_pointer_focus(&self.aggregates.hit_tester(render_geometry))?;

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

        let presenter = InstancePresenter {
            state: InstancePresenterState::WaitingForPrimaryView,
            // Correctness: We animate from 0,0 if no originating exist. Need a position here.
            center_translation_animation: scene.animated(
                originating_presenter
                    .map(|op| op.center_translation_animation.value())
                    .unwrap_or_default(),
            ),
        };

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

        self.aggregates
            .remove_target(&DesktopTarget::Instance(instance))?;
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

        Ok(())
    }

    fn hide_view(&mut self, path: ViewPath) -> Result<()> {
        let Some(instance_presenter) = self.aggregates.instances.get_mut(&path.instance) else {
            warn!("Can't hide view: Instance for view not found");
            // Robustness: Decide if this should return an error.
            return Ok(());
        };

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

        // We remove the instance for now so that we don't keep dangling references to Handle<>
        // types and be sure that they are sent to the renderer in the Desktop.
        // self.aggregates
        //     .remove_target(&DesktopTarget::Instance(path.instance))?;
        // self.aggregates.instances.remove(&path.instance)?;

        // And remove the view.
        self.aggregates
            .remove_target(&DesktopTarget::View(path.view))?;

        Ok(())
    }

    fn apply_layout(&mut self, layout: Layout<DesktopTarget, 2>, animate: bool) {
        layout.place_inline(PointPx::origin(), |id, rect_px: RectPx| {
            let rect: Rect = rect_px.into();

            self.aggregates.rects.insert(id.clone(), rect);

            match id {
                DesktopTarget::Desktop => {}
                DesktopTarget::Instance(instance_id) => {
                    self.aggregates
                        .instances
                        .get_mut(&instance_id)
                        .expect("Instance missing")
                        .set_rect(rect_px, animate);
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
            };
        });
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

    pub fn forward_event_transitions(
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
    pub fn forward_event_transition(
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
                if let Err(e) = instance_manager.send_view_event((instance, view_id), event.clone())
                {
                    // This is not an error we want to stop the world for now.
                    warn!("Sending view event {event:?} failed with {e}");
                }
            }
            DesktopTarget::Group(..) => {}
            DesktopTarget::Launcher(launcher_id) => {
                // Architecture: Shouldn't we move the hovering into the launcher presenters or even into the system?
                match event {
                    ViewEvent::CursorEntered { .. } => {
                        let launcher = &self.aggregates.launchers[&launcher_id];
                        let rect = launcher.rect.final_value();
                        self.aggregates.project_presenter.show_hover_rect(rect);
                    }
                    ViewEvent::CursorLeft { .. } => {
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
            DesktopTarget::Desktop => {
                Some(self.aggregates.rects[&DesktopTarget::Desktop].to_camera())
            }

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

        let from_rect = self.aggregates.rects.get(from)?;
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
            .map(|t| (t.clone(), self.aggregates.rects[&t]));

        let ordered =
            ordered_rects_in_direction(from_rect.center(), direction, navigation_candidates);
        if let Some((nearest, _rect)) = ordered.first() {
            return Some(nearest.clone());
        }
        None
    }
}

impl Aggregates {
    pub fn hit_tester<'a>(&'a self, geometry: &'a RenderGeometry) -> AggregateHitTester<'a> {
        AggregateHitTester {
            aggregates: self,
            geometry,
        }
    }

    /// Remove the target from the hierarchy and rects. Specific target aggregates are left
    /// untouched (they may be needed for fading out, etc.).
    pub fn remove_target(&mut self, target: &DesktopTarget) -> Result<()> {
        self.hierarchy.remove(target)?;
        self.rects.remove(target);
        Ok(())
    }

    pub fn view_of_instance(&self, instance: InstanceId) -> Option<ViewId> {
        let nested = self.hierarchy.get_nested(&instance.into());
        if let [DesktopTarget::View(view)] = nested {
            Some(*view)
        } else {
            None
        }
    }
}

struct AggregateHitTester<'a> {
    aggregates: &'a Aggregates,
    geometry: &'a RenderGeometry,
}

impl AggregateHitTester<'_> {
    fn hit_test_hierarchy(
        &self,
        screen_pos: Point,
        root: &DesktopTarget,
    ) -> Option<(DesktopTarget, Vector3)> {
        let rect = self.aggregates.rects.get(root)?;
        let model = Matrix4::IDENTITY;
        let local_pos = self.geometry.unproject_to_model_z0(screen_pos, &model)?;
        let point = Point::new(local_pos.x, local_pos.y);
        if rect.contains(point) {
            // Prefer nested hits over container hits.
            for nested in self.aggregates.hierarchy.get_nested(root) {
                if let Some(target_hit) = self.hit_test_hierarchy(screen_pos, nested) {
                    return Some(target_hit);
                }
            }
            // No nested hit, container hits.
            return Some((root.clone(), local_pos));
        }

        None
    }

    fn hit_test_target_plane(&self, screen_pos: Point) -> Option<Vector3> {
        // let rect = self.aggregates.rects.get(target)?;
        let model = Matrix4::IDENTITY;
        self.geometry.unproject_to_model_z0(screen_pos, &model)
    }
}

impl HitTester<DesktopTarget> for AggregateHitTester<'_> {
    fn hit_test(
        &self,
        screen_pos: massive_geometry::Point,
        target: Option<&DesktopTarget>,
    ) -> Option<(DesktopTarget, massive_geometry::Vector3)> {
        match target {
            Some(target) => self
                .hit_test_target_plane(screen_pos)
                .map(|hit| (target.clone(), hit)),
            None => self.hit_test_hierarchy(screen_pos, &DesktopTarget::Desktop),
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
