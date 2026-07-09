use std::{sync::Arc, time::Duration};

use massive_animation::{Animated, Interpolation};
use massive_geometry::{Color, Point, Rect, Size, SizePx, Transform};
use massive_layout::{Placement, Rect as LayoutRect};
use massive_renderer::text::FontSystem;
use massive_scene::{
    At, Handle, IntoVisual, Location, Object, StageIdentityLocation, ToLocationRelative, Visual,
};
use massive_shapes::{self as shapes, IntoShape, Shape, Size as SizeExt, StrokeRect};
use massive_shell::Scene;

use super::ProjectProperties;

const PROJECT_HEADER_FONT_SIZE: f32 = 16.0 * 8.0;
const PROJECT_HEADER_BACKGROUND_COLOR: Color = Color::rgb_u32(0x1f4d3d);
const PROJECT_HEADER_BACKGROUND_ALPHA: f32 = 0.65;
const PROJECT_HEADER_TEXT_COLOR: Color = Color::WHITE;
const PROJECT_HEADER_TEXT_DECAL_ORDER: usize = 0;
const PROJECT_HEADER_ANIMATION_DURATION: Duration = Duration::from_millis(500);

/// Presents project-level visuals and scene anchors.
///
/// Responsibilities:
/// - Provides the shared parent location for launcher and instance presenters.
/// - Presents the project's hover outline visual.
#[derive(Debug)]
pub struct DesktopPresenter {
    pub location: Handle<Location>,

    // Idea: Use a type that combines Alpha with another `Interpolatable` type.
    // Robustness: Alpha should be a type.
    hover_alpha: Animated<f32>,
    hover_placement: Placement<Transform, 2>,
    hover_scene_transform: Handle<Transform>,
    hover_location: Handle<Location>,
    // Idea: can't we just animate a visual / Handle<Visual>?
    // Performance: This is a visual that _always_ lives inside the renderer, even though it does not contain a single shape when alpha = 0.0
    hover_visual: Handle<Visual>,
    hover_placement_cache: Option<Placement<Transform, 2>>,
}

impl DesktopPresenter {
    const HOVER_STROKE: (f64, f64) = (10.0, 10.0);

    pub fn new(location: Handle<Location>, scene: &Scene) -> Self {
        let (hover_scene_transform, hover_location) = scene.stage_identity_location();

        Self {
            location: location.clone(),
            hover_alpha: scene.animated(0.0),
            hover_placement: Placement::new(Transform::IDENTITY, LayoutRect::EMPTY),
            hover_scene_transform,
            hover_location: hover_location.clone(),
            hover_visual: create_hover_shapes(None)
                .into_visual()
                .at(hover_location)
                .enter(scene),
            hover_placement_cache: None,
        }
    }

    const HOVER_ANIMATION_DURATION: Duration = Duration::from_millis(250);

    pub fn set_hover_placement(&mut self, placement: Option<Placement<Transform, 2>>) {
        if self.hover_placement_cache == placement {
            return;
        }
        self.hover_placement_cache = placement;

        match placement {
            Some(placement) => {
                self.hover_alpha.animate_if_changed(
                    1.0,
                    Self::HOVER_ANIMATION_DURATION,
                    Interpolation::CubicOut,
                );

                self.hover_placement = placement;

                let alpha = *self.hover_alpha.value();
                self.update_hover_placement_and_visual(placement, alpha);
            }

            None => {
                self.hover_alpha.animate_if_changed(
                    0.0,
                    Self::HOVER_ANIMATION_DURATION,
                    Interpolation::CubicOut,
                );
            }
        }
        // Placement changes must reach the scene even when the alpha animation is already idle.
        // self.apply_animations();
    }

    pub fn apply_animations(&mut self) {
        let alpha = *self.hover_alpha.value();
        self.update_hover_placement_and_visual(self.hover_placement, alpha);
    }

    fn update_hover_placement_and_visual(&self, placement: Placement<Transform, 2>, alpha: f32) {
        let size = placement.rect.size;
        let local_rect = Rect::from_size((size[0] as f64, size[1] as f64));
        let rect_alpha = (alpha != 0.0).then_some((local_rect, alpha));

        // Position the hover visual in world space using the placement's center-based transform.
        let local_center = local_rect.center();
        let scene_transform = placement
            .transform
            .to_origin_space(Point::new(local_center.x, local_center.y));
        self.hover_scene_transform
            .update_if_changed(scene_transform);

        // Ergonomics: What something like `apply_to_if_changed(&mut self.hover_visual)` or so?
        //
        // Performance: Can't be update just the shapes here with apply...
        let visual = create_hover_shapes(rect_alpha)
            .into_visual()
            .at(&self.hover_location)
            .with_decal_order(5);
        self.hover_visual.update_if_changed(visual);
    }
}

fn create_hover_shapes(rect_alpha: Option<(Rect, f32)>) -> Arc<[Shape]> {
    rect_alpha
        .map(|(r, a)| {
            let stroke = DesktopPresenter::HOVER_STROKE;
            StrokeRect {
                rect: r.with_outset(stroke),
                stroke: stroke.into(),
                color: Color::rgb_u32(0xff0000).with_alpha(a),
            }
            .into()
        })
        .into_iter()
        .collect()
}

#[derive(Debug)]
pub struct ProjectPresenter {
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
        let header = ProjectHeaderPresenter::new(properties, location.clone(), scene, font_system);
        let matrix = ProjectMatrixPresenter::new(location.clone(), scene);

        Self {
            scene_transform,
            header,
            matrix,
        }
    }

    pub fn set_layout(&mut self, size: SizePx, layout_transform: Transform) {
        let scene_transform =
            layout_transform.to_origin_space_from_size(size.width as f64, size.height as f64);
        self.scene_transform.update_if_changed(scene_transform);
    }

    pub fn apply_animations(&mut self) {
        self.header.apply_animations();
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
            animated_size: scene.animated(Size::default()),
            measured_size,
            scene_transform,
            background,
            name,
        }
    }

    pub fn measured_size(&self) -> SizePx {
        self.measured_size
    }

    pub fn set_layout(&mut self, size: SizePx, layout_transform: Transform, animate: bool) {
        self.layout_transform = layout_transform;
        let size = Size::new(size.width as f64, size.height as f64);

        if animate {
            self.animated_size.animate_if_changed(
                size,
                PROJECT_HEADER_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        } else {
            self.animated_size.set_immediately(size);
            self.apply_animations();
        }
    }

    pub fn apply_animations(&mut self) {
        let size = self.animated_size.value();
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
