use std::time::{Duration, Instant};

use anyhow::Result;
use winit::event::MouseButton;
use winit::keyboard::{Key, NamedKey};

use massive_animation::{Animated, Interpolation};
use massive_applications::{InstanceId, ViewEvent};
use massive_geometry::{Color, Quaternion, Rect, RectPx, SizePx, Vector3};
use massive_input::EventManager;
use massive_layout::{LayoutAxis, Offset, Size as LayoutSize};
use massive_renderer::text::FontSystem;
use massive_scene::{At, Handle, Location, Object, ToLocation, ToTransform, Transform, Visual};
use massive_shapes::{self as shapes, IntoShape, Shape, Size};
use massive_shell::Scene;

use super::visor_layout;
use crate::desktop_system::{Cmd, DesktopCommand, place_container_children};
use crate::projects::LaunchProfileId;

use super::configuration::{LaunchProfile, LauncherMode};

// TODO: Need proper color palettes for UI elements.
// const ALICE_BLUE: Color = Color::rgb_u32(0xf0f8ff);
// const POWDER_BLUE: Color = Color::rgb_u32(0xb0e0e6);
const MIDNIGHT_BLUE: Color = Color::rgb_u32(0x191970);

const BACKGROUND_COLOR: Color = MIDNIGHT_BLUE;
const TEXT_COLOR: Color = Color::WHITE;
const FADING_DURATION: Duration = Duration::from_millis(500);

const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy)]
pub struct LauncherInstanceLayoutInput {
    pub instance_id: InstanceId,
    pub rect: RectPx,
}

#[derive(Debug, Clone, Copy)]
pub struct LauncherInstanceLayoutTarget {
    pub instance_id: InstanceId,
    pub rect: RectPx,
    pub layout_transform: Transform,
}

#[derive(Debug)]
pub struct LauncherPresenter {
    #[allow(unused)]
    id: LaunchProfileId,
    profile: LaunchProfile,
    mode: LauncherMode,
    transform: Handle<Transform>,

    pub rect: Animated<Rect>,
    background: Handle<Visual>,
    // The text, either centered, or on top of the border.
    name: Handle<Visual>,

    // Alpha fading of name / background.
    fader: Animated<f32>,

    events: EventManager<ViewEvent>,
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
        let mode = profile.mode;

        let our_transform = rect.origin().to_transform().enter(scene);

        let our_location = our_transform
            .to_location()
            .relative_to(&parent_location)
            .enter(scene);

        let background = background_shape.at(&our_location).enter(scene);

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
            .with_decal_order(0)
            .enter(scene);

        Self {
            id,
            profile,
            mode,
            transform: our_transform,
            rect: scene.animated(rect),
            background,
            name,
            fader: scene.animated(1.0),
            events: EventManager::default(),
        }
    }

    pub fn should_render_instance_background(&self) -> bool {
        self.mode == LauncherMode::Visor
    }

    pub fn includes_overflow_children_in_hit_testing(&self) -> bool {
        matches!(self.mode, LauncherMode::Visor)
    }

    pub fn compute_instance_layout_targets(
        &self,
        instances: &[LauncherInstanceLayoutInput],
        focused_instance: Option<InstanceId>,
    ) -> Vec<LauncherInstanceLayoutTarget> {
        match self.mode {
            LauncherMode::Band => self.flat_layout_targets(instances),
            LauncherMode::Visor if instances.len() <= 1 => self.flat_layout_targets(instances),
            LauncherMode::Visor => self.visor_layout_targets(instances, focused_instance),
        }
    }

    pub fn panel_measure_size(&self, default_panel_size: SizePx) -> Option<LayoutSize<2>> {
        match self.mode {
            LauncherMode::Band => None,
            LauncherMode::Visor => Some(default_panel_size.into()),
        }
    }

    pub fn panel_child_offsets(
        &self,
        parent_offset: Offset<2>,
        child_sizes: &[LayoutSize<2>],
        default_panel_size: SizePx,
    ) -> Option<Vec<Offset<2>>> {
        match self.mode {
            LauncherMode::Band => None,
            LauncherMode::Visor => Some(centered_horizontal_offsets(
                parent_offset,
                child_sizes,
                default_panel_size.width as i32,
            )),
        }
    }

    pub fn should_relayout_on_focus_change(&self, instance_count: usize) -> bool {
        matches!(self.mode, LauncherMode::Visor) && instance_count > 1
    }

    fn visor_layout_targets(
        &self,
        instances: &[LauncherInstanceLayoutInput],
        focused_instance: Option<InstanceId>,
    ) -> Vec<LauncherInstanceLayoutTarget> {
        debug_assert!(matches!(self.mode, LauncherMode::Visor));

        let focused_index = focused_instance
            .and_then(|focused| instances.iter().position(|i| i.instance_id == focused));

        let first_center = instances
            .first()
            .expect("Internal error: Expected at least one instance")
            .rect
            .center()
            .cast::<f64>()
            .x;
        let last_center = instances
            .last()
            .expect("Internal error: Expected at least one instance")
            .rect
            .center()
            .cast::<f64>()
            .x;

        let group_center_x = (first_center + last_center) * 0.5;
        let flat_span = (last_center - first_center).abs();

        instances
            .iter()
            .enumerate()
            .map(|(index, input)| {
                let placement =
                    visor_layout::placement(index, instances.len(), flat_span, focused_index)
                        .expect("Internal error: Visor placement requires at least two instances");

                let center = input.rect.center().cast::<f64>();
                let center_translation = Vector3::new(
                    group_center_x + placement.center_x_offset,
                    center.y,
                    placement.center_z,
                );
                let layout_transform = Transform::new(
                    center_translation,
                    Quaternion::from_rotation_y(placement.yaw),
                    1.0,
                );

                LauncherInstanceLayoutTarget {
                    instance_id: input.instance_id,
                    rect: input.rect,
                    layout_transform,
                }
            })
            .collect()
    }

    fn flat_layout_targets(
        &self,
        instances: &[LauncherInstanceLayoutInput],
    ) -> Vec<LauncherInstanceLayoutTarget> {
        instances
            .iter()
            .map(|input| {
                let center = input.rect.center().cast::<f64>();
                let center_translation = Vector3::new(center.x, center.y, 0.0);

                LauncherInstanceLayoutTarget {
                    instance_id: input.instance_id,
                    rect: input.rect,
                    layout_transform: Transform::from_translation(center_translation),
                }
            })
            .collect()
    }

    // Architecture: I don't want the launcher here to directly generate commands. may be
    // LauncherCommand? Not sure.
    pub fn process(&mut self, view_event: ViewEvent) -> Result<Cmd> {
        let presents_instance = self.presents_instance();

        // Architecture: This looks horrible, what about just hiding ExternalEvent and passing each
        // member (also make the scope type optional, generic over the EventManager?).
        let Some(input_event) = self.events.add_event(view_event, Instant::now()) else {
            return Ok(Cmd::None);
        };

        if presents_instance {
            return Ok(Cmd::None);
        }

        // Can't go on focus here, we might focus launchers by other means (for example cursor
        // navigation).
        if input_event.detect_click(MouseButton::Left).is_some() {
            // Usability: Should pass this rect?
            return Ok(DesktopCommand::StartInstance {
                launcher: self.id,
                parameters: self.profile.params.clone(),
            }
            .into());
        }

        if input_event.event().pressed_key() == Some(&Key::Named(NamedKey::Enter))
            && input_event.keyboard_modifiers().super_key()
        {
            return Ok(DesktopCommand::StartInstance {
                launcher: self.id,
                parameters: self.profile.params.clone(),
            }
            .into());
        }

        Ok(Cmd::None)
    }

    fn presents_instance(&self) -> bool {
        self.fader.final_value() == 0.0
    }

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
    }

    pub fn fade_out(&mut self) {
        self.fader
            .animate(0.0, FADING_DURATION, Interpolation::CubicOut);
    }

    pub fn fade_in(&mut self) {
        self.fader
            .animate(1.0, FADING_DURATION, Interpolation::CubicOut);
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
    }
}

fn background_shape(rect: Rect, color: Color) -> Shape {
    shapes::Rect::new(rect, color).into()
}

fn centered_horizontal_offsets(
    parent_offset: Offset<2>,
    child_sizes: &[LayoutSize<2>],
    panel_width: i32,
) -> Vec<Offset<2>> {
    let spacing = 0i32;
    let children_span: i32 = child_sizes.iter().map(|size| size[0] as i32).sum::<i32>()
        + spacing * (child_sizes.len().saturating_sub(1) as i32);
    let center_offset = (panel_width - children_span) / 2;

    let mut offset = parent_offset;
    offset[0] += center_offset;

    place_container_children(LayoutAxis::HORIZONTAL, spacing, offset, child_sizes)
}
