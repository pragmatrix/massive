use std::time::Duration;

use massive_animation::{Animated, AnimationAllocator, AnimationTimeProvider, Interpolation};
use massive_geometry::{Color, Rect, Size, SizePx, Transform};
use massive_renderer::text::FontSystem;
use massive_scene::{At, Handle, Location, Object, ToLocationRelative, Visual};
use massive_shapes::{self as shapes, IntoShape, Shape, Size as SizeExt};
use massive_shell::Scene;

use super::ProjectProperties;

const PROJECT_HEADER_FONT_SIZE: f32 = 16.0 * 8.0;
const PROJECT_HEADER_BACKGROUND_COLOR: Color = Color::rgb_u32(0x1f4d3d);
const PROJECT_HEADER_BACKGROUND_ALPHA: f32 = 0.65;
const PROJECT_HEADER_TEXT_COLOR: Color = Color::WHITE;
const PROJECT_HEADER_TEXT_DECAL_ORDER: usize = 0;
const PROJECT_HEADER_ANIMATION_DURATION: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub struct ProjectPresenter {
    name: String,
    scene_transform: Handle<Transform>,
    pub header: ProjectHeaderPresenter,
    pub matrix: ProjectMatrixPresenter,
}

impl ProjectPresenter {
    pub fn new(
        properties: ProjectProperties,
        parent_location: Handle<Location>,
        scene: &Scene,
        font_system: &mut FontSystem,
    ) -> Self {
        let scene_transform = Transform::IDENTITY.enter(scene);
        let location = scene_transform
            .to_location_relative(&parent_location)
            .enter(scene);
        let name = properties.name.clone();
        let header = ProjectHeaderPresenter::new(properties, location.clone(), scene, font_system);
        let matrix = ProjectMatrixPresenter::new(location.clone(), scene);

        Self {
            name,
            scene_transform,
            header,
            matrix,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_layout(&mut self, size: SizePx, layout_transform: Transform) {
        let scene_transform =
            layout_transform.to_origin_space_from_size(size.width as f64, size.height as f64);
        self.scene_transform.update_if_changed(scene_transform);
    }

    pub fn apply_animations(&mut self, context: &dyn AnimationTimeProvider) {
        self.header.apply_animations(context);
    }
}

#[derive(Debug)]
pub struct ProjectHeaderPresenter {
    layout_transform: Transform,
    animated_size: Animated<Size>,
    measured_size: SizePx,
    scene_transform: Handle<Transform>,
    background: Handle<Visual>,
    name: Handle<Visual>,
}

impl ProjectHeaderPresenter {
    pub fn new(
        properties: ProjectProperties,
        parent_location: Handle<Location>,
        scene: &Scene,
        font_system: &mut FontSystem,
    ) -> Self {
        let scene_transform = Transform::IDENTITY.enter(scene);
        let location = scene_transform
            .to_location_relative(&parent_location)
            .enter(scene);

        // Architecture: It may be preferable to allow empty glyph runs for invalid/empty names.
        let header_run = properties
            .name
            .size(PROJECT_HEADER_FONT_SIZE)
            .shape(font_system);
        let measured_size = header_run
            .as_ref()
            .map_or(SizePx::default(), |run| run.metrics.size());

        let background = background_shape(Rect::default(), PROJECT_HEADER_BACKGROUND_COLOR)
            .at(&location)
            .enter(scene);

        let name = header_run
            .map(|run| run.with_color(PROJECT_HEADER_TEXT_COLOR).into_shape())
            .at(&location)
            .with_decal_order(PROJECT_HEADER_TEXT_DECAL_ORDER)
            .enter(scene);

        Self {
            layout_transform: Transform::IDENTITY,
            animated_size: Size::default().into(),
            measured_size,
            scene_transform,
            background,
            name,
        }
    }

    pub fn measured_size(&self) -> SizePx {
        self.measured_size
    }

    pub fn set_layout(
        &mut self,
        context: &mut dyn AnimationAllocator,
        size: SizePx,
        layout_transform: Transform,
        animate: bool,
    ) {
        self.layout_transform = layout_transform;
        let size = Size::new(size.width as f64, size.height as f64);

        if animate {
            self.animated_size.animate_if_changed(
                context,
                size,
                PROJECT_HEADER_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        } else {
            self.animated_size.snap(size);
            self.apply_animations(context.time_provider());
        }
    }

    pub fn apply_animations(&mut self, context: &dyn AnimationTimeProvider) {
        let size = self.animated_size.value(context);
        let scene_transform = self
            .layout_transform
            .to_origin_space_from_size(size.width, size.height);
        self.scene_transform.update_if_changed(scene_transform);
        self.background.update_if_changed_with(|visual| {
            visual.shapes = [background_shape(
                size.to_rect(),
                PROJECT_HEADER_BACKGROUND_COLOR.with_alpha(PROJECT_HEADER_BACKGROUND_ALPHA),
            )]
            .into()
        });
        self.name.update_if_changed_with(|visual| {
            visual.shapes = match &*visual.shapes {
                [Shape::GlyphRun(gr)] => [gr
                    .clone()
                    .with_color(PROJECT_HEADER_TEXT_COLOR)
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

#[derive(Debug)]
pub struct ProjectMatrixPresenter {
    pub size: SizePx,
    scene_transform: Handle<Transform>,
    location: Handle<Location>,
}

impl ProjectMatrixPresenter {
    pub fn new(parent_location: Handle<Location>, scene: &Scene) -> Self {
        let scene_transform = Transform::IDENTITY.enter(scene);
        let location = scene_transform
            .to_location_relative(&parent_location)
            .enter(scene);

        Self {
            size: SizePx::default(),
            scene_transform,
            location,
        }
    }

    pub fn location(&self) -> Handle<Location> {
        self.location.clone()
    }

    pub fn set_layout(&mut self, size: SizePx, layout_transform: Transform) {
        self.size = size;
        let scene_transform =
            layout_transform.to_origin_space_from_size(size.width as f64, size.height as f64);
        self.scene_transform.update_if_changed(scene_transform);
    }
}
