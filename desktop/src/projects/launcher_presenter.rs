use std::time::{Duration, Instant};

use anyhow::Result;
use winit::event::MouseButton;
use winit::keyboard::{Key, NamedKey};

use massive_animation::{Animated, Interpolation};
use massive_applications::{InstanceId, ViewEvent};
use massive_geometry::{Color, Quaternion, Rect, RectPx, Size, SizePx, Vector3};
use massive_input::EventManager;
use massive_layout::{Offset, Rect as LayoutRect, Size as LayoutSize, TransformOffset};
use massive_renderer::text::FontSystem;
use massive_scene::{At, Handle, Location, Object, ToLocation, Transform, Visual};
use massive_shapes::{self as shapes, IntoShape, Shape, Size as SizeExt};
use massive_shell::Scene;

use super::visor_layout;
use crate::desktop_system::{Cmd, DesktopCommand};
use crate::instance_presenter::InstancePresenter;
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
const CHILD_SPACING: i32 = 0;

#[derive(Debug, Clone, Copy)]
struct VisorLayoutSummary {
    group_center_x: f64,
    flat_span: f64,
    focused_index: Option<usize>,
    instance_count: usize,
}

#[derive(Debug)]
pub struct LauncherPresenter {
    #[allow(unused)]
    id: LaunchProfileId,
    profile: LaunchProfile,
    mode: LauncherMode,
    layout_transform: Transform,
    scene_transform: Handle<Transform>,

    pub size: Animated<Size>,
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
        size: Size,
        scene: &Scene,
        font_system: &mut FontSystem,
    ) -> Self {
        // Ergonomics: I want this to look like rect.as_shape().with_color(Color::WHITE);
        let background_shape = background_shape(size.to_rect(), BACKGROUND_COLOR);
        let mode = profile.mode;

        let our_transform = Transform::IDENTITY.enter(scene);

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
            layout_transform: Transform::IDENTITY,
            scene_transform: our_transform,
            size: scene.animated(size),
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

    pub fn place_panel_children(
        &self,
        parent_offset: Offset<2>,
        child_sizes: &[LayoutSize<2>],
        child_instances: impl IntoIterator<Item = Option<InstanceId>>,
        default_panel_size: SizePx,
        focused_instance: Option<InstanceId>,
    ) -> Vec<TransformOffset<Transform, 2>> {
        let child_instances: Vec<_> = child_instances.into_iter().collect();
        debug_assert_eq!(child_sizes.len(), child_instances.len());

        let children_span = child_sizes.iter().map(|size| size[0] as i32).sum::<i32>()
            + CHILD_SPACING * child_sizes.len().saturating_sub(1) as i32;

        let mut offset = parent_offset;
        if matches!(self.mode, LauncherMode::Visor) {
            offset[0] += (default_panel_size.width as i32 - children_span) / 2;
        }

        let visor_layout_summary = if matches!(self.mode, LauncherMode::Visor) {
            let mut first_center_x = None;
            let mut last_center_x = None;
            let mut focused_index = None;
            let mut instance_count = 0;
            let mut child_offset = offset;

            for (index, (&child_size, instance_id)) in
                child_sizes.iter().zip(&child_instances).enumerate()
            {
                if index > 0 {
                    child_offset[0] += CHILD_SPACING;
                }

                if let Some(instance_id) = instance_id {
                    let rect: RectPx = LayoutRect::new(child_offset, child_size).into();
                    let center_x = rect.center().cast::<f64>().x;
                    first_center_x.get_or_insert(center_x);
                    last_center_x = Some(center_x);

                    if Some(*instance_id) == focused_instance {
                        focused_index = Some(instance_count);
                    }

                    instance_count += 1;
                }

                child_offset[0] += child_size[0] as i32;
            }

            if instance_count > 1 {
                let first_center_x =
                    first_center_x.expect("Internal error: Expected at least one instance");
                let last_center_x =
                    last_center_x.expect("Internal error: Expected at least one instance");

                Some(VisorLayoutSummary {
                    group_center_x: (first_center_x + last_center_x) * 0.5,
                    flat_span: (last_center_x - first_center_x).abs(),
                    focused_index,
                    instance_count,
                })
            } else {
                None
            }
        } else {
            None
        };

        let mut child_placements = Vec::with_capacity(child_sizes.len());
        let mut instance_index = 0;

        for (index, (&child_size, instance_id)) in
            child_sizes.iter().zip(child_instances).enumerate()
        {
            if index > 0 {
                offset[0] += CHILD_SPACING;
            }

            let rect: RectPx = LayoutRect::new(offset, child_size).into();
            let center = rect.center().cast::<f64>();
            let transform = if instance_id.is_some() {
                if let Some(summary) = visor_layout_summary {
                    let placement = visor_layout::placement(
                        instance_index,
                        summary.instance_count,
                        summary.flat_span,
                        summary.focused_index,
                    )
                    .expect("Internal error: Visor placement requires at least two instances");
                    instance_index += 1;

                    Transform::new(
                        Vector3::new(
                            summary.group_center_x + placement.center_x_offset,
                            center.y,
                            placement.center_z,
                        ),
                        Quaternion::from_rotation_y(placement.yaw),
                        1.0,
                    )
                } else {
                    instance_index += 1;
                    Transform::from_translation(Vector3::new(center.x, center.y, 0.0))
                }
            } else {
                Transform::from_translation(Vector3::new(center.x, center.y, 0.0))
            };

            child_placements.push(TransformOffset::new(transform, offset));
            offset[0] += child_size[0] as i32;
        }

        child_placements
    }

    pub fn panel_measure_size(&self, default_panel_size: SizePx) -> Option<LayoutSize<2>> {
        match self.mode {
            LauncherMode::Band => None,
            LauncherMode::Visor => Some(default_panel_size.into()),
        }
    }

    pub fn should_relayout_on_focus_change(&self, instance_count: usize) -> bool {
        matches!(self.mode, LauncherMode::Visor) && instance_count > 1
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

    pub fn set_layout(&mut self, size: SizePx, layout_transform: Transform, animate: bool) {
        self.layout_transform = layout_transform;
        let size = Size::new(size.width as f64, size.height as f64);
        if animate {
            self.size.animate_if_changed(
                size,
                STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        } else {
            self.size.set_immediately(size);
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
        let size = self.size.value();
        let local_center = size.to_rect().center();

        let scene_transform =
            InstancePresenter::transform_with_layout(self.layout_transform, local_center);
        self.scene_transform.update_if_changed(scene_transform);

        let alpha = self.fader.value();

        // Performance: How can we not call this if self.size and self.fader are both not animating.
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
