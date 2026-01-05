use std::collections::HashMap;

use massive_animation::Animated;
use massive_geometry::{Color, Rect};
use massive_scene::{Handle, Location, Visual};
use massive_shapes as shapes;
use massive_shell::Scene;

use crate::projects::launch_group::{GroupId, LaunchGroup, LauncherId};

#[derive(Debug)]
struct ProjectPresenter {
    location: Handle<Location>,
    /// The current hierarchy root, directly derived from the configuration. This is for layout. It
    /// references the presenters through GroupIds and SlotIds.
    root: LaunchGroup,

    groups: HashMap<GroupId, GroupPresenter>,
    // Naming: Find a better name for Slot
    slots: HashMap<LauncherId, SlotPresenter>,
}

impl ProjectPresenter {
    pub fn new(root: LaunchGroup, location: Handle<Location>) -> Self {
        Self {
            location,
            root,
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
