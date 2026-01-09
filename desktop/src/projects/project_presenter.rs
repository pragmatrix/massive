use std::{
    collections::{HashMap, hash_map},
    time::Duration,
};

use massive_animation::{Animated, Interpolation};
use massive_geometry::{Color, Rect, RectPx, SizePx};
use massive_layout::{LayoutInfo, LayoutNode, layout};
use massive_scene::{Handle, Location, Visual};
use massive_shapes as shapes;
use massive_shell::Scene;

use crate::projects::{
    Project,
    project::{GroupId, LaunchGroup, LaunchGroupContents, LaunchProfileId, Launcher},
};

const ANIMATION_DURATION: Duration = Duration::from_millis(500);

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

    // pub fn layout(&mut self, default_size: SizePx, scene: &Scene) {
    //     let mut context = LayoutContext {
    //         default_size,
    //         presenter: self,
    //         scene,
    //     };

    //     layout(&mut self.project.root, &mut context);
    // }

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

    fn set_launch_rect(&mut self, id: LaunchProfileId, rect: RectPx, scene: &Scene) {
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

#[derive(Debug)]
struct LayoutContext<'a> {
    default_size: SizePx,
    presenter: &'a mut ProjectPresenter,
    scene: &'a Scene,
}

impl<'a> LayoutNode<LayoutContext<'a>> for LaunchGroup {
    type Rect = RectPx;

    fn layout_info(&self, _context: &LayoutContext) -> LayoutInfo<SizePx> {
        LayoutInfo::Container {
            child_count: self.contents.len(),
            layout_axis: self.layout.axis(),
        }
    }

    fn get_child_mut(
        &mut self,
        index: usize,
    ) -> &mut dyn LayoutNode<LayoutContext<'a>, Rect = RectPx> {
        match &mut self.contents {
            LaunchGroupContents::Groups(launch_groups) => &mut launch_groups[index],
            LaunchGroupContents::Launchers(launchers) => &mut launchers[index],
        }
    }

    fn set_rect(&mut self, rect: Self::Rect, context: &mut LayoutContext) {
        context
            .presenter
            .set_group_rect(self.id, rect, context.scene);
    }
}

impl LayoutNode<LayoutContext<'_>> for Launcher {
    type Rect = RectPx;

    fn layout_info(&self, context: &LayoutContext) -> LayoutInfo<SizePx> {
        LayoutInfo::Leaf {
            size: context.default_size,
        }
    }

    fn set_rect(&mut self, rect: Self::Rect, context: &mut LayoutContext) {
        context
            .presenter
            .set_launch_rect(self.id, rect, context.scene);
    }
}
