use std::time::{Duration, Instant};

use anyhow::Result;
use uuid::Uuid;
use winit::event::MouseButton;
use winit::keyboard::{Key, NamedKey};

use massive_animation::{Animated, Interpolation};
use massive_applications::{InstanceId, ViewEvent};
use massive_geometry::{Color, Quaternion, Rect, RectPx, Size, SizePx, Vector3};
use massive_input::EventManager;
use massive_layout::{LayoutAxis, Offset, Placement, Rect as LayoutRect, Size as LayoutSize};
use massive_renderer::text::FontSystem;
use massive_scene::{At, Handle, Location, Object, ToLocationRelative, Transform, Visual};
use massive_shapes::{self as shapes, IntoShape, Shape, Size as SizeExt};
use massive_shell::Scene;

use super::visor_layout;
use crate::Map;
use crate::desktop_system::{Commands, DesktopCommand, place_container_children};
use crate::instance_presenter::InstancePresenter;
use crate::projects::{LaunchProfileId, MatrixPlacement};

use super::configuration::{LaunchProfile, LauncherMode};

// TODO: Need proper color palettes for UI elements.
// spellcheck: ignore
// const ALICE_BLUE: Color = Color::rgb_u32(0xf0f8ff);
// const POWDER_BLUE: Color = Color::rgb_u32(0xb0e0e6);
const MIDNIGHT_BLUE: Color = Color::rgb_u32(0x191970);

const BACKGROUND_COLOR: Color = MIDNIGHT_BLUE;
const TEXT_COLOR: Color = Color::WHITE;
const FADING_DURATION: Duration = Duration::from_millis(500);

const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);
const COLLAPSED_NON_ANCHOR_Z_OFFSET: f64 = 1.0;
const CHILD_SPACING: i32 = 0;

#[derive(Debug, Clone, Copy)]
struct VisorLayoutSummary {
    group_center_x: f64,
    flat_span: f64,
    instance_count: usize,
}

#[derive(Debug)]
pub struct LauncherPresenter {
    #[allow(unused)]
    id: LaunchProfileId,
    profile: LaunchProfile,
    pub placement: MatrixPlacement,
    mode: LauncherMode,
    layout_transform: Transform,
    scene_transform: Handle<Transform>,
    location: Handle<Location>,

    pub size: Animated<Size>,
    background: Handle<Visual>,
    // The text, either centered, or on top of the border.
    name: Handle<Visual>,

    // Alpha fading of name / background.
    fader: Animated<f32>,
    /// The visor's focus anchor the visor centers on and that stays visible during collapse: the
    /// most recently focused instance while no mouse button was pressed. The visor centers on this
    /// anchor independent of the live keyboard focus.
    pub focus_anchor_instance: Option<InstanceId>,

    /// We need our own EventManager, because the event's positions are relative to us and even if
    /// they weren't sharing the event history isn't really possible.
    event_manager: EventManager<ViewEvent>,
}

impl LauncherPresenter {
    // Ergonomics: Scene can be imported from two locations, use just the shell one, or somehow
    // introduce something new that exports more ergonomic UI components.
    pub fn new(
        parent_location: Handle<Location>,
        id: LaunchProfileId,
        placement: MatrixPlacement,
        profile: LaunchProfile,
        size: Size,
        scene: &Scene,
        font_system: &mut FontSystem,
    ) -> Self {
        // Ergonomics: I want this to look like `rect.as_shape().with_color(Color::WHITE);`
        let background_shape = background_shape(size.to_rect(), BACKGROUND_COLOR);
        let mode = profile.mode;

        let our_transform = Transform::IDENTITY.enter(scene);
        let our_location = our_transform
            .to_location_relative(&parent_location)
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
            .at(&our_location)
            .with_decal_order(0)
            .enter(scene);

        Self {
            id,
            profile,
            placement,
            mode,
            layout_transform: Transform::IDENTITY,
            scene_transform: our_transform,
            location: our_location,
            size: scene.animated(size),
            background,
            name,
            fader: scene.animated(1.0),
            focus_anchor_instance: None,
            event_manager: EventManager::default(),
        }
    }

    pub fn should_render_instance_background(&self) -> bool {
        match self.mode {
            LauncherMode::Band => false,
            LauncherMode::Visor => true,
        }
    }

    pub fn includes_overflow_children_in_hit_testing(&self) -> bool {
        match self.mode {
            LauncherMode::Band => false,
            LauncherMode::Visor => true,
        }
    }

    pub fn place_panel_children(
        &self,
        local_offset: Offset<2>,
        child_sizes: &[LayoutSize<2>],
        child_instances: &[InstanceId],
        expanded: bool,
        default_panel_size: SizePx,
    ) -> Vec<Placement<Transform, 2>> {
        match self.mode {
            LauncherMode::Band => place_container_children(
                LayoutAxis::HORIZONTAL,
                CHILD_SPACING,
                local_offset,
                child_sizes,
            ),
            LauncherMode::Visor => {
                let center_index = self
                    .focus_anchor_instance
                    .and_then(|anchor| {
                        child_instances
                            .iter()
                            .position(|&instance| instance == anchor)
                    })
                    .unwrap_or_default();

                self.place_visor_panel_children(
                    local_offset,
                    child_sizes,
                    center_index,
                    expanded,
                    default_panel_size,
                )
            }
        }
    }

    fn place_visor_panel_children(
        &self,
        local_offset: Offset<2>,
        child_sizes: &[LayoutSize<2>],
        center_index: usize,
        expanded: bool,
        default_panel_size: SizePx,
    ) -> Vec<Placement<Transform, 2>> {
        let offset =
            centered_children_offset(local_offset, child_sizes, default_panel_size.width as i32);

        let Some(summary) = visor_layout_summary(offset, child_sizes) else {
            return place_container_children(
                LayoutAxis::HORIZONTAL,
                CHILD_SPACING,
                offset,
                child_sizes,
            );
        };

        let mut child_placements = Vec::with_capacity(child_sizes.len());
        let mut offset = offset;

        for (child_index, &child_size) in child_sizes.iter().enumerate() {
            if child_index > 0 {
                offset[0] += CHILD_SPACING;
            }

            let center_y = child_center_y(offset, child_size);
            let transform =
                visor_child_transform(child_index, center_y, summary, center_index, expanded);
            let visible = visor_child_visibility(child_index, center_index, expanded);

            child_placements.push(
                Placement::new(transform, LayoutRect::new(offset, child_size))
                    .with_visibility(visible),
            );
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

    pub fn should_relayout_on_keyboard_focus_change(&self, instance_count: usize) -> bool {
        matches!(self.mode, LauncherMode::Visor) && instance_count > 1
    }

    pub fn mode(&self) -> LauncherMode {
        self.mode
    }

    // Architecture: I don't want the launcher here to directly generate commands. may be
    // LauncherCommand? Not sure.
    pub fn process(&mut self, event: ViewEvent) -> Result<Commands> {
        let presents_instance = self.presents_instance();

        let Some(event) = self.event_manager.add_event(event, Instant::now()) else {
            return Ok(Commands::Empty);
        };

        if presents_instance {
            return Ok(Commands::Empty);
        }

        // Can't go on focus here, we might focus launchers by other means (for example cursor
        // navigation).
        let start_instance = event.detect_click(MouseButton::Left).is_some()
            || (event.event().pressed_key() == Some(&Key::Named(NamedKey::Enter))
                && event.keyboard_modifiers().super_key());

        if start_instance {
            // Usability: Should pass this rectangle?
            return Ok(DesktopCommand::StartInstance {
                launcher: self.id,
                instance: Uuid::new_v4().into(),
                root: None,
                parameters: self.profile.params.clone(),
            }
            .into());
        }

        Ok(Commands::Empty)
    }

    fn presents_instance(&self) -> bool {
        // Robustness: Deriving this state from crossing into upper layer (state -> animation).
        *self.fader.final_value() == 0.0
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
            self.apply_presenter_animations();
        }
    }

    pub fn location(&self) -> Handle<Location> {
        self.location.clone()
    }

    pub fn fade_out(&mut self) {
        self.fader
            .animate(0.0, FADING_DURATION, Interpolation::CubicOut);
    }

    pub fn fade_in(&mut self) {
        self.fader
            .animate(1.0, FADING_DURATION, Interpolation::CubicOut);
    }

    pub fn apply_animations(
        &mut self,
        instances: &mut Map<InstanceId, InstancePresenter>,
        child_instances: &[InstanceId],
    ) {
        self.apply_presenter_animations();
        // I think this does not make sense, we can do this externally (going over all instances)
        self.apply_child_instance_animations(instances, child_instances);
    }

    fn apply_presenter_animations(&mut self) {
        let size = self.size.value();

        let scene_transform = self
            .layout_transform
            .to_origin_space_from_size(size.width, size.height);
        self.scene_transform.update_if_changed(scene_transform);

        let alpha = self.fader.value();

        // Performance: How can we not call this if `self.size` and `self.fader` are both not
        // animating. `is_animating()` is perhaps not reliable.
        self.background.update_if_changed_with(|visual| {
            visual.shapes = [background_shape(
                size.to_rect(),
                BACKGROUND_COLOR.with_alpha(*alpha),
            )]
            .into()
        });

        // Ergonomics: Isn't there a better way to directly set new shapes?
        self.name.update_if_changed_with(|visual| {
            visual.shapes = match &*visual.shapes {
                [Shape::GlyphRun(gr)] => [gr
                    .clone()
                    .with_color(TEXT_COLOR.with_alpha(*alpha))
                    .into_shape()]
                .into(),
                rest => rest.into(),
            }
        });
    }

    fn apply_child_instance_animations(
        &mut self,
        instances: &mut Map<InstanceId, InstancePresenter>,
        child_instances: &[InstanceId],
    ) {
        for instance_id in child_instances {
            instances
                .get_mut(instance_id)
                .expect("Instance missing")
                .apply_animations();
        }
    }
}

fn background_shape(rect: Rect, color: Color) -> Shape {
    shapes::Rect::new(rect, color).into()
}

fn centered_children_offset(
    local_offset: Offset<2>,
    child_sizes: &[LayoutSize<2>],
    panel_width: i32,
) -> Offset<2> {
    let mut offset = local_offset;
    offset[0] += (panel_width - children_span(child_sizes)) / 2;
    offset
}

fn children_span(child_sizes: &[LayoutSize<2>]) -> i32 {
    child_sizes.iter().map(|size| size[0] as i32).sum::<i32>()
        + CHILD_SPACING * child_sizes.len().saturating_sub(1) as i32
}

fn visor_layout_summary(
    mut offset: Offset<2>,
    child_sizes: &[LayoutSize<2>],
) -> Option<VisorLayoutSummary> {
    let mut first_center_x = None;
    let mut last_center_x = None;

    for (child_index, &child_size) in child_sizes.iter().enumerate() {
        if child_index > 0 {
            offset[0] += CHILD_SPACING;
        }

        let center_x = child_rect(offset, child_size).center().cast::<f64>().x;
        first_center_x.get_or_insert(center_x);
        last_center_x = Some(center_x);

        offset[0] += child_size[0] as i32;
    }

    let instance_count = child_sizes.len();
    if instance_count <= 1 {
        return None;
    }

    let first_center_x = first_center_x.expect("Internal error: Expected at least one instance");
    let last_center_x = last_center_x.expect("Internal error: Expected at least one instance");

    Some(VisorLayoutSummary {
        group_center_x: (first_center_x + last_center_x) * 0.5,
        flat_span: (last_center_x - first_center_x).abs(),
        instance_count,
    })
}

fn child_rect(offset: Offset<2>, child_size: LayoutSize<2>) -> RectPx {
    LayoutRect::new(offset, child_size).into()
}

fn child_center_y(offset: Offset<2>, child_size: LayoutSize<2>) -> f64 {
    (offset[1] + child_size[1] as i32 / 2) as f64
}

fn visor_child_transform(
    instance_index: usize,
    center_y: f64,
    summary: VisorLayoutSummary,
    center_index: usize,
    expanded: bool,
) -> Transform {
    let expansion_factor = if expanded { 1.0 } else { 0.0 };
    let placement = visor_layout::placement(
        instance_index,
        summary.instance_count,
        summary.flat_span,
        center_index,
        expansion_factor,
    )
    .expect("Internal error: Visor placement requires at least two instances");
    let mut transform = Transform::new(
        Vector3::new(
            summary.group_center_x + placement.center_x_offset,
            center_y,
            placement.center_z,
        ),
        Quaternion::from_rotation_y(placement.yaw),
        1.0,
    );

    if instance_index != center_index {
        transform.translate.z += COLLAPSED_NON_ANCHOR_Z_OFFSET * if expanded { 0.0 } else { 1.0 };
    }

    transform
}

fn visor_child_visibility(instance_index: usize, center_index: usize, expanded: bool) -> bool {
    expanded || instance_index == center_index
}
