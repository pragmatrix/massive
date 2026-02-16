use std::time::{Duration, Instant};

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

use crate::desktop_system::{Cmd, DesktopCommand};
use crate::projects::LaunchProfileId;

use super::STRUCTURAL_ANIMATION_DURATION;
use super::configuration::LaunchProfile;

// TODO: Need proper color palettes for UI elements.
// const ALICE_BLUE: Color = Color::rgb_u32(0xf0f8ff);
// const POWDER_BLUE: Color = Color::rgb_u32(0xb0e0e6);
const MIDNIGHT_BLUE: Color = Color::rgb_u32(0x191970);

const BACKGROUND_COLOR: Color = MIDNIGHT_BLUE;
const TEXT_COLOR: Color = Color::WHITE;
const FADING_DURATION: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub struct LauncherPresenter {
    pub id: LaunchProfileId,
    profile: LaunchProfile,
    transform: Handle<Transform>,
    // location: Handle<Location>,
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

    /// The instances.
    // band: BandPresenter,

    // Alpha fading of name / background.
    fader: Animated<f32>,
}

impl LauncherPresenter {
    // Ergonomics: Scene can be imported from two locations, use just the shell one, or somehow
    // introduce something new that exports more ergonomic UI components.

    pub fn new(
        parent_location: Handle<Location>,
        id: LaunchProfileId,
        profile: LaunchProfile,
        rect: Rect,
        scene: &Scene,
        font_system: &mut FontSystem,
    ) -> Self {
        // Ergonomics: I want this to look like rect.as_shape().with_color(Color::WHITE);
        let background_shape = background_shape(rect.size().to_rect(), BACKGROUND_COLOR);

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
            .map(|r| r.with_color(TEXT_COLOR).into_shape())
            .at(our_location)
            .with_depth_bias(3)
            .enter(scene);

        Self {
            id,
            profile,
            transform: our_transform,
            // location: parent_location,
            rect: scene.animated(rect),
            background,
            name,
            events: EventManager::default(),
            // band: BandPresenter::default(),
            fader: scene.animated(1.0),
        }
    }

    // Architecture: I don't want the launcher here to directly generate commands. may be
    // LauncherCommand? Not sure.
    pub fn process(&mut self, view_event: ViewEvent) -> Result<Cmd> {
        if !self.presents_instance()
            && let ViewEvent::Focused(true) = view_event
        {
            // Usability: Should pass this rect?
            return Ok(DesktopCommand::StartInstance {
                parameters: self.profile.params.clone(),
            }
            .into());
        }

        Ok(Cmd::None)
    }

    fn presents_instance(&self) -> bool {
        self.fader.final_value() == 0.0
    }

    // pub fn process_band(&mut self, view_event: ViewEvent) -> Result<Cmd> {
    //     self.band.process(view_event).map(|()| Cmd::None)
    // }

    pub fn set_rect(&mut self, rect: Rect, animate: bool) {
        if animate {
            self.rect.animate_if_changed(
                rect,
                STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        } else {
            self.rect.set_immediately(rect);
            self.apply_animations();
        }

        // self.layout_band(true);
    }

    // pub fn is_presenting_instance(&self, instance: InstanceId) -> bool {
    //     self.band.presents_instance(instance)
    // }

    pub fn fade_out(&mut self) {
        self.fader
            .animate(0.0, FADING_DURATION, Interpolation::CubicOut);
    }

    pub fn fade_in(&mut self) {
        self.fader
            .animate(1.0, FADING_DURATION, Interpolation::CubicOut);
    }

    // pub fn present_view(&mut self, instance: InstanceId, view: &ViewCreationInfo) -> Result<()> {
    //     self.band.present_view(instance, view)?;

    //     // self.layout_band(false);
    //     Ok(())
    // }

    // pub fn hide_view(&mut self, view: ViewPath) -> Result<()> {
    //     self.band.hide_view(view)
    // }

    // pub fn set_instance_rect(&mut self, instance: InstanceId, rect: RectPx) {
    //     self.band.set_instance_rect(instance, rect, true);
    // }

    // fn layout_band(&mut self, animate: bool) {
    //     // Layout the band's instances.

    //     let band_layout = self.band.layout();
    //     let r: PointPx = self.rect.final_value().origin().to_pixels();

    //     band_layout.place_inline(r, |instance_id, bx| {
    //         self.band.set_instance_rect(instance_id, bx, animate);
    //     });
    // }

    pub fn apply_animations(&mut self) {
        let (origin, size) = self.rect.value().origin_and_size();

        self.transform.update_if_changed(origin.with_z(0.0).into());

        let alpha = self.fader.value();

        // Performance: How can we not call this if self.rect and self.fader are both not animating.
        // `is_animating()` is perhaps not reliable.
        self.background.update_with_if_changed(|visual| {
            visual.shapes = [background_shape(
                size.to_rect(),
                BACKGROUND_COLOR.with_alpha(alpha),
            )]
            .into()
        });

        // Ergonomics: Isn't there a better way to directly set new shapes?
        self.name.update_with_if_changed(|visual| {
            visual.shapes = match &*visual.shapes {
                [Shape::GlyphRun(gr)] => [gr
                    .clone()
                    .with_color(TEXT_COLOR.with_alpha(alpha))
                    .into_shape()]
                .into(),
                rest => rest.into(),
            }
        });

        // Robustness: Forgot to forward this once. How can we make sure that animations are
        // always applied if needed?
        // self.band.apply_animations();
    }
}

fn background_shape(rect: Rect, color: Color) -> Shape {
    shapes::Rect::new(rect, color).into()
}
