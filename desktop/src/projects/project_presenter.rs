use std::collections::HashMap;

use massive_animation::Animated;
use massive_geometry::{Color, Rect, RectPx, Size, SizePx};
use massive_layout::{LayoutAxis, LayoutInfo, LayoutNode};
use massive_scene::{Handle, Location, Visual};
use massive_shapes as shapes;
use massive_shell::Scene;

use crate::projects::{
    Project,
    project::{GroupId, LaunchGroup, LaunchGroupContents, LaunchProfileId, Launcher},
};

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
}

#[derive(Debug)]
#[allow(dead_code)]
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
}

#[derive(Debug)]
struct LayoutContext<'a> {
    default_size: SizePx,
    presenter: &'a mut ProjectPresenter,
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
        todo!();
    }
}

impl LayoutNode<LayoutContext<'_>> for Launcher {
    type Rect = RectPx;

    fn layout_info(&self, context: &LayoutContext) -> LayoutInfo<SizePx> {
        LayoutInfo::Leaf {
            size: context.default_size,
        }
    }
}
