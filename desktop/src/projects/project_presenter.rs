use std::{
    collections::{HashMap, hash_map},
    time::{Duration, Instant},
};

use anyhow::Result;
use derive_more::From;

use log::warn;
use massive_animation::{Animated, Interpolation};
use massive_applications::{ViewEvent, ViewId};
use massive_geometry::{Color, PixelCamera, PointPx, Rect, RectPx, SizePx};
use massive_input::{EventManager, ExternalEvent};
use massive_layout::{Box, LayoutAxis};
use massive_renderer::text::FontSystem;
use massive_scene::{
    At, Handle, Location, Object, ToCamera, ToLocation, ToTransform, Transform, Visual,
};
use massive_shapes::{self as shapes, IntoShape, Shape, Size};
use massive_shell::Scene;
use winit::event::MouseButton;

use crate::{
    EventTransition,
    navigation::{NavigationNode, container, leaf},
    projects::{
        Project,
        configuration::LaunchProfile,
        project::{GroupId, LaunchGroup, LaunchGroupContents, LaunchProfileId, Launcher},
    },
};

const ANIMATION_DURATION: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, From)]
enum LayoutId {
    Group(GroupId),
    Launcher(LaunchProfileId),
}

/// Architecture: Can't we just use inner as the root, thus preventing the lifetime here.
type Layouter<'a> = massive_layout::Layouter<'a, LayoutId, 2>;

#[derive(Debug, Clone, PartialEq, From)]
pub enum Id {
    Group(GroupId),
    Launcher(LaunchProfileId),
}

#[derive(Debug)]
pub struct ProjectPresenter {
    /// The project hierarchy is used for layout. It references the presenters through GroupIds and
    /// SlotIds.
    project: Project,

    location: Handle<Location>,

    groups: HashMap<GroupId, GroupPresenter>,
    launchers: HashMap<LaunchProfileId, LauncherPresenter>,
}

impl ProjectPresenter {
    pub fn new(project: Project, location: Handle<Location>) -> Self {
        Self {
            location,
            project,
            // Groups and slots are created when layouted.
            groups: Default::default(),
            launchers: Default::default(),
        }
    }

    pub fn layout(&mut self, default_size: SizePx, scene: &Scene, font_system: &mut FontSystem) {
        let mut layout = Layouter::root(self.project.root.id.into(), LayoutAxis::HORIZONTAL);

        layout_launch_group(&mut layout, &self.project.root, default_size);
        layout.place_inline(PointPx::zero(), |(id, rect)| {
            let rect = box_to_rect(rect);
            match id {
                LayoutId::Group(group_id) => {
                    self.set_group_rect(group_id, rect, scene);
                }
                LayoutId::Launcher(launch_profile_id) => {
                    self.set_launcher_rect(launch_profile_id, rect, scene, font_system);
                }
            }
        });
    }

    pub fn handle_event_transition(&mut self, event_transition: EventTransition<Id>) -> Result<()> {
        match event_transition {
            EventTransition::Send(focus_path, view_event) => {
                if let Some(id) = focus_path.last() {
                    match id {
                        Id::Group(group_id) => self
                            .groups
                            .get_mut(group_id)
                            .expect("Internal Error: Missing group")
                            .handle_event(view_event)?,
                        Id::Launcher(launch_profile_id) => self
                            .launchers
                            .get_mut(launch_profile_id)
                            .expect("Internal Error: Missing launcher")
                            .handle_event(view_event)?,
                    }
                }
            }
            EventTransition::Broadcast(view_event) => {
                for group in self.groups.values_mut() {
                    group.handle_event(view_event.clone())?;
                }
                for launcher in self.launchers.values_mut() {
                    launcher.handle_event(view_event.clone())?;
                }
            }
        }
        Ok(())
    }

    // Architecture: layout() has to be called before the navigation structure can return anything.
    // Perhaps manifest this in a better constructor.
    pub fn navigation(&self) -> NavigationNode<'_, Id> {
        let rect = self.groups[&self.project.root.id].rect.final_value();
        container(self.project.root.id, || {
            let mut r = Vec::new();
            match &self.project.root.contents {
                LaunchGroupContents::Groups(launch_groups) => {
                    for launch_group in launch_groups.iter() {
                        r.push(self.group_navigation(launch_group));
                    }
                }
                // Robustness: Is this true, if so, can't we create a better Project type that reflects that?
                _ => panic!("Project root must be groups"),
            }
            r
        })
        .with_rect(rect)
    }

    pub fn group_navigation<'a>(&'a self, launch_group: &'a LaunchGroup) -> NavigationNode<'a, Id> {
        let rect = self.groups[&launch_group.id].rect.final_value();
        container(launch_group.id, || {
            let mut r = Vec::new();
            match &launch_group.contents {
                LaunchGroupContents::Groups(launch_groups) => {
                    for lg in launch_groups {
                        r.push(self.group_navigation(lg))
                    }
                }
                LaunchGroupContents::Launchers(launchers) => {
                    for launcher in launchers {
                        let presenter = &self.launchers[&launcher.id];
                        r.push(presenter.navigation(launcher));
                    }
                }
            }
            r
        })
        .with_rect(rect)
    }

    // layout callbacks
    // Ergonomics: Make Scene Clone.

    fn set_group_rect(&mut self, id: GroupId, rect: RectPx, scene: &Scene) {
        use hash_map::Entry;
        let rect = rect.cast().into();
        match self.groups.entry(id) {
            Entry::Occupied(mut entry) => entry.get_mut().set_rect(rect),
            Entry::Vacant(entry) => {
                entry.insert(GroupPresenter::new(self.location.clone(), rect, scene));
            }
        }
    }

    fn set_launcher_rect(
        &mut self,
        id: LaunchProfileId,
        rect: RectPx,
        scene: &Scene,
        font_system: &mut FontSystem,
    ) {
        use hash_map::Entry;
        let profile = self
            .project
            .get_launch_profile(id)
            .expect("Internal Error: Launch profile not found");
        let rect = rect.cast().into();
        match self.launchers.entry(id) {
            Entry::Occupied(mut entry) => entry.get_mut().set_rect(rect),
            Entry::Vacant(entry) => {
                entry.insert(LauncherPresenter::new(
                    self.location.clone(),
                    profile.clone(),
                    rect,
                    scene,
                    font_system,
                ));
            }
        }
    }

    pub fn apply_animations(&mut self) {
        self.groups
            .values_mut()
            .for_each(|gp| gp.apply_animations());
        self.launchers
            .values_mut()
            .for_each(|sp| sp.apply_animations());
    }

    pub fn outer_camera(&self) -> PixelCamera {
        let root_group = self.project.root.id;
        if let Some(group) = self.groups.get(&root_group) {
            return group.camera();
        }
        Transform::IDENTITY.to_camera()
    }
}

fn box_to_rect(([x, y], [w, h]): Box<2>) -> RectPx {
    RectPx::new((x, y).into(), (w as i32, h as i32).into())
}

fn layout_launch_group(layout: &mut Layouter, group: &LaunchGroup, default_size: SizePx) {
    match &group.contents {
        LaunchGroupContents::Groups(launch_groups) => {
            for group in launch_groups {
                let mut container = layout
                    .container(group.id.into(), group.layout.axis())
                    .spacing(10)
                    .padding([10, 10], [10, 10]);
                layout_launch_group(&mut container, group, default_size);
            }
        }
        LaunchGroupContents::Launchers(launchers) => {
            for launcher in launchers {
                layout.leaf(launcher.id.into(), default_size)
            }
        }
    }
}

#[derive(Debug)]
struct GroupPresenter {
    // Ergonomics: Use just Location.
    location: Handle<Location>,
    rect: Animated<Rect>,

    background: Handle<Visual>,
}

impl GroupPresenter {
    pub fn new(location: Handle<Location>, rect: Rect, scene: &Scene) -> Self {
        // Ergonomics: I want this to look like rect.as_shape().with_color(Color::WHITE);
        //
        // Ergonomics: I need more named color constants for faster prototyping.
        let background_shape = background_shape(rect, Color::rgb_u32(0x0000ff));

        Self {
            location: location.clone(),
            rect: scene.animated(rect),
            background: [background_shape].at(&location).enter(scene),
        }
    }

    fn handle_event(&mut self, view_event: ViewEvent) -> Result<()> {
        Ok(())
    }

    fn set_rect(&mut self, rect: Rect) {
        self.rect
            .animate_if_changed(rect, ANIMATION_DURATION, Interpolation::CubicOut);
    }

    fn apply_animations(&mut self) {
        // Ergonomics: Support value_mut() (wrap the mutex guard).
        let rect = self.rect.value();
        self.background
            .update_with(|v| v.shapes = [background_shape(rect, Color::rgb_u32(0x0000ff))].into());
    }

    fn camera(&self) -> PixelCamera {
        let rect = self.rect.final_value();

        rect.center()
            .to_transform()
            .to_camera()
            .with_size(rect.size())
    }
}

#[derive(Debug)]
struct LauncherPresenter {
    profile: LaunchProfile,
    transform: Handle<Transform>,
    location: Handle<Location>,
    rect: Animated<Rect>,

    background: Handle<Visual>,
    // border: Handle<Visual>,

    // name_rect: Animated<Box>,
    // The text, either centered, or on top of the border.
    name: Handle<Visual>,

    /// Architecture: We don't want a history per presenter. What we want is a global one, but one
    /// that takes local coordinate spaces (and interaction spaces / CursorEnter / Exits) into
    /// account.
    events: EventManager<ViewEvent>,
}

impl LauncherPresenter {
    // Ergonomics: Scene can be imported from two locations, use just the shell one, or somehow
    // introduce something new that exports more ergonomic UI components.

    pub fn new(
        parent_location: Handle<Location>,
        profile: LaunchProfile,
        rect: Rect,
        scene: &Scene,
        font_system: &mut FontSystem,
    ) -> Self {
        // Ergonomics: I want this to look like rect.as_shape().with_color(Color::WHITE);
        let background_shape = background_shape(rect.size().to_rect(), Color::WHITE);

        let our_transform = rect.origin().to_transform().enter(scene);

        let our_location = our_transform
            .to_location()
            .relative_to(&parent_location)
            .enter(scene);

        let background = background_shape
            .at(&our_location)
            .with_depth_bias(1)
            .enter(scene);

        let name = profile
            .name
            // Idea: To not waste so much memory here for large fonts, may use a quality index that
            // is automatically applied based on the font, small fonts high quality, large fonts,
            // lower quality, the quality index starts with 1 and is the effective pixel resolution
            // divisor: Quality 1: original size, quality 2: 1/4th the memory in use (horizontal
            // size / 2, vertical size / 2)
            //
            // Idea: No, this should be fully automatic depending of how large the font is shown I
            // guess. Make this independent of the font size, but dependent on what is visible (a
            // background optimizer).
            .size(32.0 * 8.0)
            .shape(font_system)
            .map(|r| r.into_shape())
            .at(our_location)
            .with_depth_bias(3)
            .enter(scene);

        Self {
            profile,
            transform: our_transform,
            location: parent_location,
            rect: scene.animated(rect),
            background,
            name,
            events: EventManager::default(),
        }
    }

    fn navigation(&self, launcher: &Launcher) -> NavigationNode<'_, Id> {
        leaf(launcher.id, self.rect.final_value())
    }

    fn handle_event(&mut self, view_event: ViewEvent) -> Result<()> {
        // Architecture: Need something other than predefined scope if we want to reuse ViewEvent in
        // arbitrary hierarchies? May be the EventManager directly defines the scope id?
        // Ergonomics: Create a fluent constructor for events with Scope?
        let Some(event) = self.events.add_event(ExternalEvent::new(
            uuid::Uuid::nil().into(),
            view_event,
            Instant::now(),
        )) else {
            return Ok(());
        };

        if let Some(point) = event.detect_click(MouseButton::Left) {
            warn!("CLICKED on {point:?}");
        }

        match event.event() {
            ViewEvent::CursorEntered { .. } => {
                warn!("CursorEntered: {}", self.profile.name);
            }
            ViewEvent::CursorLeft { .. } => {
                warn!("CursorLeft   : {}", self.profile.name);
            }
            _ => {}
        }

        Ok(())
    }

    fn set_rect(&mut self, rect: Rect) {
        self.rect
            .animate_if_changed(rect, ANIMATION_DURATION, Interpolation::CubicOut);
    }

    fn apply_animations(&mut self) {
        let (origin, size) = self.rect.value().origin_and_size();

        self.transform.update_if_changed(origin.with_z(0.0).into());

        self.background.update_with(|visual| {
            visual.shapes = [background_shape(size.to_rect(), Color::WHITE)].into()
        });
    }
}

fn background_shape(rect: Rect, color: Color) -> Shape {
    shapes::Rect::new(rect, color).into()
}
