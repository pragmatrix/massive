//! The Desktop as an event sourced user interface system.
//!
//! The presenter hierarchy is treated as an aggregate built up from the events.
//!
//! The decision to use event sourcing stems from the fact that we want to run everything as
//! incrementally as possible, because we want to add projects, etc.
//!
//! The goal here is to remove as much as possible from the specific instances into separate systems
//! and aggregates that are event driven.

use anyhow::{Result, anyhow, bail};
use derive_more::From;
use log::{error, warn};

use massive_animation::{Animated, Interpolation};
use massive_applications::{
    CreationMode, InstanceId, InstanceParameters, ViewCreationInfo, ViewEvent, ViewId, ViewRole,
};
use massive_geometry::{PixelCamera, PointPx, Rect, RectPx, SizePx};
use massive_input::Event;
use massive_layout::{Layout, LayoutAxis, Padding, Thickness, container, leaf};
use massive_renderer::{RenderGeometry, text::FontSystem};
use massive_scene::{Location, Object, ToCamera, Transform};
use massive_shell::{FontManager, Scene};
use winit::event::ElementState;
use winit::keyboard::{Key, NamedKey};

use crate::HitTester;
use crate::event_sourcing::{self, Transaction};
use crate::focus_path::FocusPath;
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::instance_presenter::{InstancePresenter, InstancePresenterState, PrimaryViewPresenter};
use crate::projects::{
    GroupId, GroupPresenter, LaunchGroupProperties, LaunchProfile, LaunchProfileId,
    LauncherPresenter, ProjectPresenter, STRUCTURAL_ANIMATION_DURATION,
};
use crate::{DesktopEnvironment, EventRouter, EventTransition, Map, OrderedHierarchy};

const SECTION_SPACING: u32 = 20;

/// This enum specifies a unique target inside the navigation and layout history.
#[derive(Debug, Clone, PartialEq, Eq, Hash, From)]
pub enum DesktopTarget {
    // The whole area, covering the top band and
    Desktop,
    TopBand,

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
    PresentInstance(InstanceId),
    PresentView(InstanceId, ViewCreationInfo),
    StartInstance { parameters: InstanceParameters },
    StopInstance(InstanceId),
    // Simplify: Use only DesktopTarget here, DesktopFocusPath can be resolved.
    Focus(DesktopFocusPath),
}

#[derive(Debug)]
pub enum ProjectCommand {
    // Project Configuration
    AddLaunchGroup {
        parent: Option<GroupId>,
        id: GroupId,
        properties: LaunchGroupProperties,
    },
    RemoveLaunchGroup(GroupId),
    AddLauncher {
        group: GroupId,
        id: LaunchProfileId,
        profile: LaunchProfile,
    },
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
    startup_profile: Option<LaunchProfileId>,

    // For hit testing.
    desktop_rect: Rect,
    top_band_rect: Rect,

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

            desktop_rect: Rect::default(),
            top_band_rect: Rect::default(),

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
        root_group: GroupId,
        default_panel_size: SizePx,
        scene: &Scene,
    ) -> Result<Self> {
        // Set up static hierarchy parts and layout specs.

        let mut hierarchy = OrderedHierarchy::default();
        hierarchy.add_nested(
            DesktopTarget::Desktop,
            [DesktopTarget::TopBand, DesktopTarget::Group(root_group)],
        )?;

        // Architecture: This is a direct requirement from the project presenter. But where does our
        // root location actually come from, shouldn't it be provided by the caller.
        let identity_matrix = Transform::IDENTITY.enter(scene);
        let location = Location::new(None, identity_matrix).enter(scene);

        let project_presenter = ProjectPresenter::new(location, scene);

        let event_router = EventRouter::default();

        let system = Self {
            env,
            fonts,
            default_panel_size,

            event_router,
            camera: scene.animated(PixelCamera::default()),

            aggregates: Aggregates::new(hierarchy, project_presenter),
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
        geometry: &RenderGeometry,
    ) -> Result<()> {
        for command in transaction.into() {
            self.apply_command(command, scene, instance_manager, geometry)?;
        }

        Ok(())
    }

    // Architecture: The current focus is part of the system, so DesktopInteraction should probably be embedded here.
    fn apply_command(
        &mut self,
        command: DesktopCommand,
        scene: &Scene,
        instance_manager: &mut InstanceManager,
        geometry: &RenderGeometry,
    ) -> Result<()> {
        match command {
            DesktopCommand::StartInstance { parameters } => {
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
                    DesktopCommand::PresentInstance(instance),
                    scene,
                    instance_manager,
                    geometry,
                )
            }

            DesktopCommand::StopInstance(instance) => {
                // Remove the instance from the focus first.
                //
                // Detail: This causes an unfocus event sent to the instance's view. I don't think
                // this should happen on teardown.
                let focus = self.event_router.focused();
                if let Some(focused_instance) = self.event_router.focused().instance()
                    && focused_instance == instance
                {
                    let instance_parent = focus.instance_parent().expect("Internal error: Instance parent failed even though instance() returned one.");
                    let intent = self.focus(instance_parent, instance_manager)?;
                    assert!(intent.is_none());
                }

                instance_manager.request_shutdown(instance)?;

                // We hide the instance as soon we request a shutdown so that they can't be in the
                // navigation tree anymore.
                self.hide_instance(instance)?;

                // Refocus the pointer since it may be pointing to the removed instance.
                let cmd = self.refocus_pointer(instance_manager, geometry)?;
                // No intent on refocusing allowed.
                assert!(cmd.is_none());

                // remove it from the hierarchy.
                self.aggregates
                    .hierarchy
                    .remove(&DesktopTarget::Instance(instance))?;

                Ok(())
            }

            DesktopCommand::PresentInstance(instance) => {
                let focused = self.event_router.focused();
                let originating_from = focused.instance();
                let instance_parent_path = focused.instance_parent().ok_or(anyhow!(
                    "Failed to present instance when no parent is focused that can take on a new one: {focused:?}"
                ))?;

                let instance_parent = instance_parent_path.last().unwrap().clone();

                let insertion_index = self.present_instance(
                    instance_parent.clone(),
                    originating_from,
                    instance,
                    scene,
                )?;

                let instance_target = DesktopTarget::Instance(instance);
                let instance_path = instance_parent_path.clone().join(instance_target.clone());

                // Add this instance to the hierarchy.
                self.aggregates.hierarchy.insert_at(
                    instance_parent,
                    insertion_index,
                    instance_target.clone(),
                )?;

                // Focus it.
                let transitions = self.event_router.focus(instance_path);
                let cmd =
                    self.forward_event_transitions(transitions.transitions, instance_manager)?;
                assert!(cmd.is_none());
                Ok(())
            }

            DesktopCommand::PresentView(instance, creation_info) => {
                self.present_view(instance, &creation_info)?;

                let focused = self.event_router.focused();
                // If this instance is currently focused and the new view is primary, make it
                // foreground so that the view is focused.
                if matches!(focused.last(), Some(DesktopTarget::Instance(i)) if *i == instance)
                    && creation_info.role == ViewRole::Primary
                {
                    let view_focus = focused.clone().join(DesktopTarget::View(creation_info.id));
                    let cmd = self.focus(view_focus, instance_manager)?;
                    assert!(cmd.is_none())
                }

                Ok(())
            }

            DesktopCommand::Project(project_command) => {
                self.apply_project_command(project_command, scene)
            }

            DesktopCommand::Focus(path) => {
                assert!(self.focus(path, instance_manager)?.is_none());
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
                if let Some(parent) = parent {
                    self.aggregates.hierarchy.add(parent.into(), id.into())?;
                };
                self.aggregates
                    .groups
                    .insert(id, GroupPresenter::new(properties))?;
            }
            ProjectCommand::RemoveLaunchGroup(group) => {
                let target = group.into();
                self.aggregates.hierarchy.remove(&target)?;
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

                let target = DesktopTarget::Launcher(id);
                self.aggregates
                    .hierarchy
                    .add(group.into(), target.clone())?;
            }
            ProjectCommand::RemoveLauncher(id) => {
                let target = DesktopTarget::Launcher(id);
                self.aggregates.hierarchy.remove(&target)?;

                self.aggregates.launchers.remove(&id)?;
            }
            ProjectCommand::SetStartupProfile(launch_profile_id) => {
                self.aggregates.startup_profile = launch_profile_id
            }
        }

        Ok(())
    }

    /// Update all effects.
    pub fn update_effects(&mut self, animate: bool) -> Result<()> {
        // Layout & Apple rects.

        let layout = self.desktop_layout();
        self.apply_layout(layout, animate);

        // Camera

        let camera = self.camera_for_focus(self.event_router.focused());
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
            DesktopTarget::TopBand => LayoutAxis::HORIZONTAL.into(),
            DesktopTarget::Group(group_id) => self.aggregates.groups[group_id]
                .properties
                .layout
                .axis()
                .to_container()
                .spacing(10)
                .padding(10, 10)
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
            DesktopTarget::Instance(_) => self.default_panel_size.into(),
            DesktopTarget::View(_) => {
                panic!("Views are not part of the layout hierarchy");
            }
        }
    }

    pub fn process_input_event(
        &mut self,
        event: &Event<ViewEvent>,
        instance_manager: &InstanceManager,
        render_geometry: &RenderGeometry,
    ) -> Result<Cmd> {
        let hit_tester = &self.aggregates.hit_tester(render_geometry);

        let transitions =
            self.event_router
                .process(event, &self.aggregates.hierarchy, hit_tester)?;

        self.forward_event_transitions(transitions.transitions, instance_manager)
    }

    pub fn focus(
        &mut self,
        focus_path: DesktopFocusPath,
        instance_manager: &InstanceManager,
    ) -> Result<Cmd> {
        let transitions = self.event_router.focus(focus_path);
        self.forward_event_transitions(transitions.transitions, instance_manager)
    }

    pub fn refocus_pointer(
        &mut self,
        instance_manager: &InstanceManager,
        render_geometry: &RenderGeometry,
    ) -> Result<Cmd> {
        let transitions = self.event_router.reset_pointer_focus(
            &self.aggregates.hierarchy,
            &self.aggregates.hit_tester(render_geometry),
        )?;

        self.forward_event_transitions(transitions.transitions, instance_manager)
    }

    fn present_instance(
        &mut self,
        instance_parent: DesktopTarget,
        originating_from: Option<InstanceId>,
        instance: InstanceId,
        scene: &Scene,
    ) -> Result<usize> {
        let originating_presenter = originating_from
            .and_then(|originating_from| self.aggregates.instances.get(&originating_from));

        let presenter = InstancePresenter {
            state: InstancePresenterState::WaitingForPrimaryView,
            panel_size: originating_presenter
                .map(|p| p.panel_size)
                .unwrap_or(self.default_panel_size),
            rect: RectPx::zero(),
            // Correctness: We animate from 0,0 if no originating exist. Need a position here.
            center_translation_animation: scene.animated(
                originating_presenter
                    .map(|op| op.center_translation_animation.value())
                    .unwrap_or_default(),
            ),
        };

        self.aggregates.instances.insert(instance, presenter)?;

        let nested = self.aggregates.hierarchy.get_nested(&instance_parent);
        let pos = if let Some(originating_from) = originating_from {
            nested
                .iter()
                .position(|i| *i == DesktopTarget::Instance(originating_from))
                .map(|i| i + 1)
                .unwrap_or(nested.len())
        } else {
            0
        };

        Ok(pos)
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
        instance_presenter.panel_size = view_creation_info.size();
        instance_presenter.state = InstancePresenterState::Presenting {
            view: PrimaryViewPresenter {
                creation_info: view_creation_info.clone(),
            },
        };

        Ok(())
    }

    pub fn hide_view(&mut self, path: ViewPath) -> Result<()> {
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
        self.aggregates.instances.remove(&path.instance)?;
        Ok(())
    }

    fn hide_instance(&mut self, instance: InstanceId) -> Result<()> {
        self.aggregates.instances.remove(&instance)
    }

    fn apply_layout(&mut self, layout: Layout<DesktopTarget, 2>, animate: bool) {
        layout.place_inline(PointPx::origin(), |id, rect_px: RectPx| match id {
            DesktopTarget::Desktop => {
                self.aggregates.desktop_rect = rect_px.into();
            }
            DesktopTarget::TopBand => {
                self.aggregates.top_band_rect = rect_px.into();
            }
            DesktopTarget::Instance(instance_id) => {
                self.aggregates
                    .instances
                    .get_mut(&instance_id)
                    .expect("Instance missing")
                    .set_rect(rect_px, animate);
            }
            DesktopTarget::Group(..) => {}
            DesktopTarget::Launcher(launcher_id) => {
                self.aggregates
                    .launchers
                    .get_mut(&launcher_id)
                    .expect("Launcher missing")
                    .set_rect(rect_px.into());
            }
            DesktopTarget::View(..) => {
                panic!("View layout isn't supported (instance target defines its size)");
            }
        });
    }

    fn preprocess_keyboard_commands(&self, event: &Event<ViewEvent>) -> Result<Cmd> {
        // Catch CMD+t and CMD+w if an instance has the keyboard focus.

        if let ViewEvent::KeyboardInput {
            event: key_event, ..
        } = event.event()
            && key_event.state == ElementState::Pressed
            && !key_event.repeat
            && event.device_states().is_command()
        {
            if let Some(instance) = self.event_router.focused().instance() {
                match &key_event.logical_key {
                    Key::Character(c) if c.as_str() == "t" => {
                        return Ok(DesktopCommand::StartInstance {
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

            if let Some(parent_focus) = self.event_router.focused().parent()
                && let Key::Named(NamedKey::Escape) = &key_event.logical_key
            {
                return Ok(DesktopCommand::Focus(parent_focus).into());
            }
        }

        Ok(Cmd::None)
    }

    pub fn forward_event_transitions(
        &mut self,
        // Don't use EventTransitions here for now, it contains more information than we need.
        transitions: Vec<EventTransition<DesktopTarget>>,
        instance_manager: &InstanceManager,
    ) -> Result<Cmd> {
        let mut cmd = Cmd::None;

        // Robustness: While we need to forward all transitions we currently process only one intent.
        for transition in transitions {
            cmd += self.forward_event_transition(transition, instance_manager)?;
        }

        Ok(cmd)
    }

    /// Forward event transitions to the appropriate handler based on the target type.
    pub fn forward_event_transition(
        &mut self,
        transition: EventTransition<DesktopTarget>,
        instance_manager: &InstanceManager,
    ) -> Result<Cmd> {
        let mut cmd = Cmd::None;
        match transition {
            EventTransition::Directed(path, _) if path.is_empty() => {
                // This happens if hit testing hits no presenter and a CursorMove event gets
                // forwarded: FocusPath::EMPTY represents the Window itself.
            }
            EventTransition::Directed(focus_path, view_event) => {
                // Route to the appropriate handler based on the last target in the path
                match focus_path.last().expect("Internal Error") {
                    DesktopTarget::Desktop => {}
                    DesktopTarget::TopBand => {}
                    DesktopTarget::Instance(..) => {}
                    DesktopTarget::View(view_id) => {
                        let Some(instance) = focus_path.instance() else {
                            bail!("Internal error: Instance of view {view_id:?} not found");
                        };
                        if let Err(e) = instance_manager
                            .send_view_event((instance, *view_id), view_event.clone())
                        {
                            // This is not an error we want to stop the world for now.
                            error!("Sending view event {view_event:?} failed with {e:?}");
                        }
                    }
                    DesktopTarget::Group(..) => {}
                    DesktopTarget::Launcher(launcher_id) => {
                        // Architecture: Shouldn't we move the hovering into the launcher presenters or even into the system?
                        match view_event {
                            ViewEvent::CursorEntered { .. } => {
                                let launcher = &self.aggregates.launchers[launcher_id];
                                let rect = launcher.rect.final_value();
                                self.aggregates.project_presenter.show_hover_rect(rect);
                            }
                            ViewEvent::CursorLeft { .. } => {
                                self.aggregates.project_presenter.hide_hover_rect();
                            }
                            view_event => {
                                let launcher = self
                                    .aggregates
                                    .launchers
                                    .get_mut(launcher_id)
                                    .expect("Launcher not found");
                                cmd += launcher.process(view_event)?;
                            }
                        }
                    }
                }
            }
            EventTransition::Broadcast(view_event) => {
                // Broadcast to all views in instance manager
                for (view_path, _) in instance_manager.views() {
                    instance_manager.send_view_event(view_path, view_event.clone())?;
                }
            }
        }
        Ok(cmd)
    }

    // Camera

    pub fn camera_for_focus(&self, focus: &DesktopFocusPath) -> Option<PixelCamera> {
        match focus.last()? {
            // Desktop and TopBand are constrained to their size.
            DesktopTarget::Desktop => Some(self.aggregates.desktop_rect.to_camera()),
            DesktopTarget::TopBand => Some(self.aggregates.top_band_rect.to_camera()),

            DesktopTarget::Instance(instance_id) => {
                let instance = &self.aggregates.instances[instance_id];
                let transform: Transform =
                    instance.center_translation_animation.final_value().into();
                Some(transform.to_camera())
            }
            DesktopTarget::View(_) => {
                // Forward this to the parent (which must be a ::Instance).
                self.camera_for_focus(&focus.parent()?)
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
        }
    }
}

#[derive(Debug, From)]
pub enum LayoutSpec {
    Container {
        axis: LayoutAxis,
        padding: Thickness<2>,
        spacing: u32,
    },
    #[from]
    Leaf(SizePx),
}

impl From<LayoutAxis> for LayoutSpec {
    fn from(axis: LayoutAxis) -> Self {
        Self::Container {
            axis,
            padding: Default::default(),
            spacing: 0,
        }
    }
}

impl From<ContainerBuilder> for LayoutSpec {
    fn from(value: ContainerBuilder) -> Self {
        LayoutSpec::Container {
            axis: value.axis,
            padding: value.padding,
            spacing: value.spacing,
        }
    }
}

#[derive(Debug)]
struct ContainerBuilder {
    axis: LayoutAxis,
    padding: Thickness<2>,
    spacing: u32,
}

impl ContainerBuilder {
    pub fn new(axis: LayoutAxis) -> Self {
        Self {
            axis,
            padding: Default::default(),
            spacing: 0,
        }
    }

    pub fn padding(
        mut self,
        leading: impl Into<Padding<2>>,
        trailing: impl Into<Padding<2>>,
    ) -> Self {
        self.padding = (leading.into(), trailing.into()).into();
        self
    }

    pub fn spacing(mut self, spacing: u32) -> Self {
        self.spacing = spacing;
        self
    }
}

// We seem to benefit from .into() and to_container() invocations. to_container is useful for
// chaining follow ups to the builder.

impl From<LayoutAxis> for ContainerBuilder {
    fn from(axis: LayoutAxis) -> Self {
        ContainerBuilder::new(axis)
    }
}

trait ToContainer {
    fn to_container(self) -> ContainerBuilder;
}

impl ToContainer for LayoutAxis {
    fn to_container(self) -> ContainerBuilder {
        ContainerBuilder::new(self)
    }
}

impl Aggregates {
    pub fn hit_tester<'a>(&'a self, geometry: &'a RenderGeometry) -> AggregateHitTester {
        AggregateHitTester {
            aggregates: self,
            geometry,
        }
    }
}

struct AggregateHitTester<'a> {
    aggregates: &'a Aggregates,
    geometry: &'a RenderGeometry,
}

impl HitTester<DesktopTarget> for AggregateHitTester<'_> {
    fn hit_test(
        &self,
        screen_pos: massive_geometry::Point,
        target: Option<&DesktopTarget>,
    ) -> Option<(DesktopTarget, massive_geometry::Vector3)> {
        todo!()
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
                DesktopTarget::TopBand => Some(i + 1),
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
