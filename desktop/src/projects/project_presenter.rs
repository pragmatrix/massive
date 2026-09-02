use std::time::Duration;

use massive_animation::{
    Animated, AnimationAllocator, AnimationProgress, Interpolation, Movement, MovementRuntime,
};
use massive_geometry::{Color, Rect, SizePx, SizedTransform, Transform};
use massive_renderer::text::FontSystem;
use massive_scene::prelude::*;
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
        movement_runtime: &mut MovementRuntime,
    ) -> Self {
        let (scene_transform, location) = identity_location()
            .relative_to(&parent_location)
            .enter(scene);
        let name = properties.name.clone();
        let header = ProjectHeaderPresenter::new(
            properties,
            location.clone(),
            scene,
            font_system,
            movement_runtime,
        );
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

    pub fn set_layout(&mut self, layout: SizedTransform) {
        let scene_transform = layout.to_origin_space();
        self.scene_transform.update_if_changed(scene_transform);
    }
}

#[derive(Debug)]
pub struct ProjectHeaderPresenter {
    measured_size: SizePx,
    movement: Movement<ProjectHeaderMovement>,
}

impl ProjectHeaderPresenter {
    pub fn new(
        properties: ProjectProperties,
        parent_location: Handle<Location>,
        scene: &Scene,
        font_system: &mut FontSystem,
        movement_runtime: &mut MovementRuntime,
    ) -> Self {
        let (scene_transform, location) = identity_location()
            .relative_to(&parent_location)
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

        let movement_scene_transform = scene_transform.clone();
        let movement_background = background.clone();
        let movement_name = name.clone();
        let movement = movement_runtime
            .movement(
                ProjectHeaderMovement::default(),
                move |movement, progress| {
                    movement.apply_animations(
                        progress,
                        &movement_scene_transform,
                        &movement_background,
                        &movement_name,
                    );
                },
            )
            .mount();

        Self {
            measured_size,
            movement,
        }
    }

    pub fn measured_size(&self) -> SizePx {
        self.measured_size
    }

    pub fn set_layout(&self, layout: SizedTransform, animate: bool) {
        self.movement.modify(move |movement, context| {
            movement.set_layout(context, layout);
        });
        if !animate {
            self.movement.snap();
        }
    }
}

#[derive(Debug)]
struct ProjectHeaderMovement {
    layout: Animated<SizedTransform>,
}

impl Default for ProjectHeaderMovement {
    fn default() -> Self {
        Self {
            layout: SizedTransform::default().into(),
        }
    }
}

impl ProjectHeaderMovement {
    fn set_layout(&mut self, context: &mut dyn AnimationAllocator, layout: SizedTransform) {
        self.layout.animate_if_changed(
            context,
            layout,
            PROJECT_HEADER_ANIMATION_DURATION,
            Interpolation::CubicOut,
        );
    }

    fn apply_animations(
        &mut self,
        progress: AnimationProgress,
        scene_transform_handle: &Handle<Transform>,
        background: &Handle<Visual>,
        name: &Handle<Visual>,
    ) {
        let layout = *self.layout.proceed(progress);
        let scene_transform = layout.to_origin_space();
        scene_transform_handle.update_if_changed(scene_transform);
        background.update_if_changed_with(|visual| {
            visual.shapes = [background_shape(
                layout.rect(),
                PROJECT_HEADER_BACKGROUND_COLOR.with_alpha(PROJECT_HEADER_BACKGROUND_ALPHA),
            )]
            .into()
        });
        name.update_if_changed_with(|visual| {
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
    scene_transform: Handle<Transform>,
    location: Handle<Location>,
}

impl ProjectMatrixPresenter {
    pub fn new(parent_location: Handle<Location>, scene: &Scene) -> Self {
        let (scene_transform, location) = identity_location()
            .relative_to(&parent_location)
            .enter(scene);

        Self {
            scene_transform,
            location,
        }
    }

    pub fn location(&self) -> Handle<Location> {
        self.location.clone()
    }

    pub fn set_layout(&mut self, layout: SizedTransform) {
        let scene_transform = layout.to_origin_space();
        self.scene_transform.update_if_changed(scene_transform);
    }
}
