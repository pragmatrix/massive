use std::{
    collections::{HashMap, hash_map},
    time::Duration,
};

use derive_more::From;

use massive_animation::{Animated, Interpolation};
use massive_geometry::{Color, PointPx, Rect, RectPx, SizePx};
use massive_layout::{Box, LayoutAxis};
use massive_renderer::text::FontSystem;
use massive_scene::{
    Handle, IntoVisual, Location, Object, ToLocation, ToTransform, Transform, Visual,
};
use massive_shapes::{self as shapes, IntoShape, Shape, Size};
use massive_shell::Scene;

use crate::projects::{
    Project,
    configuration::LaunchProfile,
    project::{GroupId, LaunchGroup, LaunchGroupContents, LaunchProfileId},
};

const ANIMATION_DURATION: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, From)]
enum LayoutId {
    Group(GroupId),
    Launcher(LaunchProfileId),
}

/// Architecture: Can't we just use inner as the root, thus preventing the lifetime here.
type Layouter<'a> = massive_layout::Layouter<'a, LayoutId, 2>;

#[derive(Debug)]
pub struct ProjectPresenter {
    /// The project hierarchy is used for layout. It references the presenters through GroupIds and
    /// SlotIds.
    project: Project,

    location: Handle<Location>,

    groups: HashMap<GroupId, GroupPresenter>,
    // Naming: Find a better name for Slot
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

    // layout callbacks
    // Ergonomics: Make Scene Clone.

    fn set_group_rect(&mut self, id: GroupId, rect: RectPx, scene: &Scene) {
        use hash_map::Entry;
        let rect = rect.cast().into();
        match self.groups.entry(id) {
            Entry::Occupied(mut entry) => entry.get_mut().set_rect(rect),
            Entry::Vacant(entry) => {
                // entry.insert(GroupPresenter::new(self.location.clone(), rect, scene));
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
            background: [background_shape].into_visual().at(&location).enter(scene),
        }
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
}

#[derive(Debug)]
struct LauncherPresenter {
    transform: Handle<Transform>,
    location: Handle<Location>,
    rect: Animated<Rect>,

    background: Handle<Visual>,
    // border: Handle<Visual>,

    // name_rect: Animated<Box>,
    // The text, either centered, or on top of the border.
    name: Handle<Visual>,
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
            .into_visual()
            .at(&our_location)
            .with_depth_bias(1)
            .enter(scene);

        let name = profile
            .name
            .size(40.0)
            .layout(font_system)
            .map(|r| r.into_shape())
            .into_visual()
            .at(our_location)
            .with_depth_bias(3)
            .enter(scene);

        Self {
            transform: our_transform,
            location: parent_location,
            rect: scene.animated(rect),
            background,
            name,
        }
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
