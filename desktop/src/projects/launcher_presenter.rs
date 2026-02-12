use std::time::{Duration, Instant};

use anyhow::Result;
use log::warn;
use winit::event::MouseButton;

use massive_animation::{Animated, Interpolation};
use massive_applications::{InstanceId, ViewCreationInfo, ViewEvent};
use massive_geometry::{Color, PointPx, Rect, SizePx, ToPixels};
use massive_input::{EventManager, ExternalEvent};
use massive_renderer::text::FontSystem;
use massive_scene::{At, Handle, Location, Object, ToLocation, ToTransform, Transform, Visual};
use massive_shapes::{self as shapes, IntoShape, Shape, Size};
use massive_shell::Scene;

use super::{ProjectTarget, STRUCTURAL_ANIMATION_DURATION, configuration::LaunchProfile};
use crate::{
    band_presenter::BandPresenter,
    desktop_system::{Cmd, DesktopCommand},
    instance_manager::ViewPath,
    navigation::{NavigationNode, leaf},
    projects::Launcher,
};

// TODO: Need proper color palettes for UI elements.
// const ALICE_BLUE: Color = Color::rgb_u32(0xf0f8ff);
// const POWDER_BLUE: Color = Color::rgb_u32(0xb0e0e6);
const MIDNIGHT_BLUE: Color = Color::rgb_u32(0x191970);

const BACKGROUND_COLOR: Color = MIDNIGHT_BLUE;
const TEXT_COLOR: Color = Color::WHITE;
const FADING_DURATION: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub struct LauncherPresenter {
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
    band: BandPresenter,

    // Alpha fading of name / background.
    fader: Animated<f32>,
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
            profile,
            transform: our_transform,
            // location: parent_location,
            rect: scene.animated(rect),
            background,
            name,
            events: EventManager::default(),
            band: BandPresenter::default(),
            fader: scene.animated(1.0),
        }
    }

    pub fn navigation(&self, launcher: &Launcher) -> NavigationNode<'_, ProjectTarget> {
        if self.band.is_empty() {
            return leaf(launcher.id, self.rect.final_value());
        }
        let launcher_id = launcher.id;
        self.band
            .navigation()
            .map_target(move |band_target| ProjectTarget::Band(launcher_id, band_target))
            .with_target(ProjectTarget::Launcher(launcher_id))
            .with_rect(self.rect.final_value())
    }

    // Architecture: I don't want the launcher here to directly generate UserIntent, may be LauncherIntent? Not sure.
    pub fn process(&mut self, view_event: ViewEvent) -> Result<Cmd> {
        // Architecture: Need something other than predefined scope if we want to reuse ViewEvent in
        // arbitrary hierarchies? May be the EventManager directly defines the scope id?
        // Ergonomics: Create a fluent constructor for events with Scope?
        let Some(event) = self.events.add_event(ExternalEvent::new(
            uuid::Uuid::nil().into(),
            view_event,
            Instant::now(),
        )) else {
            return Ok(Cmd::None);
        };

        if let Some(point) = event.detect_click(MouseButton::Left) {
            warn!("CLICKED on {point:?}");
        }

        match event.event() {
            ViewEvent::Focused(true) if self.band.is_empty() => {
                // Usability: Should pass this rect?
                return Ok(DesktopCommand::StartInstance {
                    parameters: self.profile.params.clone(),
                }
                .into());
            }
            ViewEvent::CursorEntered { .. } => {
                warn!("CursorEntered: {}", self.profile.name);
            }
            ViewEvent::CursorLeft { .. } => {
                warn!("CursorLeft   : {}", self.profile.name);
            }
            _ => {}
        }

        Ok(Cmd::None)
    }

    pub fn process_band(&mut self, view_event: ViewEvent) -> Result<Cmd> {
        self.band.process(view_event).map(|()| Cmd::None)
    }

    pub fn set_rect(&mut self, rect: Rect) {
        self.rect
            .animate_if_changed(rect, STRUCTURAL_ANIMATION_DURATION, Interpolation::CubicOut);

        self.layout_band(true);
    }

    pub fn is_presenting_instance(&self, instance: InstanceId) -> bool {
        self.band.presents_instance(instance)
    }

    pub fn present_instance(
        &mut self,
        instance: InstanceId,
        originating_from: Option<InstanceId>,
        default_panel_size: SizePx,
        scene: &Scene,
    ) -> Result<usize> {
        let was_empty = self.band.is_empty();
        let insertion_index =
            self.band
                .present_instance(instance, originating_from, default_panel_size, scene)?;
        if was_empty && !self.band.is_empty() {
            self.fader
                .animate(0.0, FADING_DURATION, Interpolation::CubicOut);
        }

        // self.layout_band(true);
        Ok(insertion_index)
    }

    pub fn hide_instance(&mut self, instance: InstanceId) -> Result<()> {
        self.band.hide_instance(instance)?;
        if self.band.is_empty() {
            self.fader
                .animate(1.0, FADING_DURATION, Interpolation::CubicOut);
        }
        Ok(())
    }

    pub fn present_view(&mut self, instance: InstanceId, view: &ViewCreationInfo) -> Result<()> {
        self.band.present_view(instance, view)?;

        // self.layout_band(false);
        Ok(())
    }

    pub fn hide_view(&mut self, view: ViewPath) -> Result<()> {
        self.band.hide_view(view)
    }

    fn layout_band(&mut self, animate: bool) {
        // Layout the band's instances.

        let band_layout = self.band.layout();
        let r: PointPx = self.rect.final_value().origin().to_pixels();

        band_layout.place_inline(r, |instance_id, bx| {
            self.band.set_instance_rect(instance_id, bx, animate);
        });
    }

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
        self.band.apply_animations();
    }
}

fn background_shape(rect: Rect, color: Color) -> Shape {
    shapes::Rect::new(rect, color).into()
}
