use std::time::Instant;

use anyhow::Result;
use log::warn;
use winit::event::MouseButton;

use massive_animation::{Animated, Interpolation};
use massive_applications::ViewEvent;
use massive_geometry::{Color, Rect};
use massive_input::{EventManager, ExternalEvent};
use massive_renderer::text::FontSystem;
use massive_scene::{At, Handle, Location, Object, ToLocation, ToTransform, Transform, Visual};
use massive_shapes::{self as shapes, IntoShape, Shape, Size};
use massive_shell::Scene;

use super::{
    ProjectTarget, STRUCTURAL_ANIMATION_DURATION, configuration::LaunchProfile, project::Launcher,
};
use crate::navigation::{NavigationNode, leaf};

#[derive(Debug)]
pub struct LauncherPresenter {
    profile: LaunchProfile,
    transform: Handle<Transform>,
    location: Handle<Location>,
    pub rect: Animated<Rect>,

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

    pub fn navigation(&self, launcher: &Launcher) -> NavigationNode<'_, ProjectTarget> {
        leaf(launcher.id, self.rect.final_value())
    }

    pub fn process(&mut self, view_event: ViewEvent) -> Result<()> {
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

    pub fn set_rect(&mut self, rect: Rect) {
        self.rect
            .animate_if_changed(rect, STRUCTURAL_ANIMATION_DURATION, Interpolation::CubicOut);
    }

    pub fn apply_animations(&mut self) {
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
