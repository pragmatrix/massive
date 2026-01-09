use std::{
    collections::{HashMap, hash_map},
    time::Duration,
};

use derive_more::From;
use massive_animation::{Animated, Interpolation};
use massive_geometry::{Color, PointPx, Rect, RectPx, SizePx};
use massive_layout::{LayoutAxis, LayoutInfo, LayoutNode, layout};
use massive_scene::{Handle, Location, Visual};
use massive_shapes as shapes;
use massive_shell::Scene;

use crate::projects::{
    Project,
    configuration::LaunchProfile,
    project::{GroupId, LaunchGroup, LaunchGroupContents, LaunchProfileId, Launcher},
};

const ANIMATION_DURATION: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, From)]
enum LayoutId {
    Group(GroupId),
    Launcher(LaunchProfileId),
}

/// Architecture: Can't we just use inner as the root, thus preventing the lifetime here.
type Layouter<'a> = massive_layout::Layouter<'a, LayoutId, RectPx>;

#[derive(Debug)]
pub struct ProjectPresenter {
    /// The project hierarchy is used for layout. It references the presenters through GroupIds and
    /// SlotIds.
    project: Project,

    location: Handle<Location>,

    groups: HashMap<GroupId, GroupPresenter>,
    // Naming: Find a better name for Slot
    slots: HashMap<LaunchProfileId, SlotPresenter>,
}

impl ProjectPresenter {
    pub fn new(project: Project, location: Handle<Location>) -> Self {
        Self {
            location,
            project,
            // Groups and slots are created when layouted.
            groups: Default::default(),
            slots: Default::default(),
        }
    }

    pub fn layout(&mut self, default_size: SizePx, scene: &Scene) {
        let mut layout = Layouter::root(self.project.root.id.into(), LayoutAxis::HORIZONTAL);

        layout_launch_group(&mut layout, &self.project.root, default_size);
        layout.place_inline(PointPx::default(), |(id, rect)| match id {
            LayoutId::Group(group_id) => {
                self.set_group_rect(group_id, rect, scene);
            }
            LayoutId::Launcher(launch_profile_id) => {
                self.set_launcher_rect(launch_profile_id, rect, scene);
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
                entry.insert(GroupPresenter::new(self.location.clone(), rect, scene));
            }
        }
    }

    fn set_launcher_rect(&mut self, id: LaunchProfileId, rect: RectPx, scene: &Scene) {
        use hash_map::Entry;
        let rect = rect.cast().into();
        match self.slots.entry(id) {
            Entry::Occupied(mut entry) => entry.get_mut().set_rect(rect),
            Entry::Vacant(entry) => {
                entry.insert(SlotPresenter::new(self.location.clone(), rect, scene));
            }
        }
    }
}

#[derive(Debug)]
struct LayoutContext<'a> {
    default_size: SizePx,
    scene: &'a Scene,
}

fn layout_launch_group(layout: &mut Layouter, group: &LaunchGroup, default_size: SizePx) {
    match &group.contents {
        LaunchGroupContents::Groups(launch_groups) => {
            for group in launch_groups {
                let mut container = layout.container(group.id.into(), group.layout.axis());
                layout_launch_group(&mut container, &group, default_size);
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
        let background_shape = shapes::Rect::new(rect, Color::rgb_u32(0x0000ff));

        let background = Visual::new(location.clone(), [background_shape.into()]);

        Self {
            location,
            rect: scene.animated(rect),
            background: scene.stage(background),
        }
    }

    fn set_rect(&mut self, rect: Rect) {
        self.rect
            .animate_if_changed(rect, ANIMATION_DURATION, Interpolation::CubicOut);
    }
}

#[derive(Debug)]
struct SlotPresenter {
    // Ergonomics: Use just Location.
    location: Handle<Location>,
    rect: Animated<Rect>,

    background: Handle<Visual>,
    // border: Handle<Visual>,

    // name_rect: Animated<Box>,
    // The text, either centered, or on top of the border.
    // name: Handle<Visual>,
}

impl SlotPresenter {
    // Ergonomics: Scene can be imported from two locations, use just the shell one, or somehow
    // introduce something new that exports more ergonomic UI components.

    pub fn new(location: Handle<Location>, rect: Rect, scene: &Scene) -> Self {
        // Ergonomics: I want this to look like rect.as_shape().with_color(Color::WHITE);
        let background_shape = shapes::Rect::new(rect, Color::WHITE);

        let background = Visual::new(location.clone(), [background_shape.into()]);

        Self {
            location,
            rect: scene.animated(rect),
            background: scene.stage(background),
        }
    }

    fn set_rect(&mut self, rect: Rect) {
        self.rect
            .animate_if_changed(rect, ANIMATION_DURATION, Interpolation::CubicOut);
    }
}
