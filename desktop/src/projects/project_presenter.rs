use std::{sync::Arc, time::Duration};

use massive_animation::{Animated, Interpolation};
use massive_geometry::{Color, Point, Rect, SizePx, Transform};
use massive_layout::{Placement, Rect as LayoutRect};
use massive_renderer::text::FontSystem;
use massive_scene::{At, Handle, IntoVisual, Location, Object, ToLocation, Visual};
use massive_shapes::{IntoShape, Shape, Size as SizeExt, StrokeRect};
use massive_shell::Scene;

use super::ProjectProperties;

const PROJECT_HEADER_TEXT_COLOR: Color = Color::WHITE;

/// Presents project-level visuals and scene anchors.
///
/// Responsibilities:
/// - Provides the shared parent location for launcher and instance presenters.
/// - Presents the project's hover outline visual.
#[derive(Debug)]
pub struct DesktopPresenter {
    pub location: Handle<Location>,

    // Idea: Use a type that combines Alpha with another Interpolatable type.
    // Robustness: Alpha should be a type.
    hover_alpha: Animated<f32>,
    hover_placement: Placement<Transform, 2>,
    hover_scene_transform: Handle<Transform>,
    hover_location: Handle<Location>,
    // Idea: can't we just animate a visual / Handle<Visual>?
    // Performance: This is a visual that _always_ lives inside the renderer, even though it does not contain a single shape when alpha = 0.0
    hover_visual: Handle<Visual>,
}

impl DesktopPresenter {
    const HOVER_STROKE: (f64, f64) = (10.0, 10.0);

    pub fn new(location: Handle<Location>, scene: &Scene) -> Self {
        let hover_scene_transform = Transform::IDENTITY.enter(scene);
        let hover_location = Location::new(None, hover_scene_transform.clone()).enter(scene);

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
        }
    }

    const HOVER_ANIMATION_DURATION: Duration = Duration::from_millis(250);

    pub fn set_hover_placement(&mut self, placement: Option<Placement<Transform, 2>>) {
        match placement {
            Some(placement) => {
                self.hover_alpha.animate_if_changed(
                    1.0,
                    Self::HOVER_ANIMATION_DURATION,
                    Interpolation::CubicOut,
                );
                self.hover_placement = placement;
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
        self.apply_animations();
    }

    pub fn apply_animations(&mut self) {
        let alpha = self.hover_alpha.value();
        let hover_placement = self.hover_placement;

        let size = hover_placement.rect.size;
        let local_rect = Rect::from_size((size[0] as f64, size[1] as f64));
        let rect_alpha = (alpha != 0.0).then_some((local_rect, alpha));

        // Position the hover visual in world space using the placement's center-based transform.
        let local_center = local_rect.center();
        let scene_transform = hover_placement
            .transform
            .to_origin_space(Point::new(local_center.x, local_center.y));
        self.hover_scene_transform
            .update_if_changed(scene_transform);

        // Ergonomics: What something like apply_to_if_changed(&mut self.hover_visual) or so?
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
    pub size: SizePx,
    scene_transform: Handle<Transform>,
    location: Handle<Location>,
}

impl ProjectPresenter {
    pub fn new(parent_location: Handle<Location>, scene: &Scene) -> Self {
        let scene_transform = Transform::IDENTITY.enter(scene);
        let location = scene_transform
            .to_location()
            .relative_to(&parent_location)
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
        let local_center = Point::new(size.width as f64 / 2.0, size.height as f64 / 2.0);
        let scene_transform = layout_transform.to_origin_space(local_center);
        self.scene_transform.update_if_changed(scene_transform);
    }
}

#[derive(Debug)]
pub struct ProjectHeaderPresenter {
    pub size: SizePx,
    measured_size: SizePx,
    scene_transform: Handle<Transform>,
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
            .to_location()
            .relative_to(&parent_location)
            .enter(scene);

        let header_run = properties
            .name
            .size(32.0 * 8.0)
            .shape(font_system)
            .expect("Project header shaping produced no glyph run");
        let measured_size = header_run.metrics.size();

        let name = header_run
            .with_color(PROJECT_HEADER_TEXT_COLOR)
            .into_shape()
            .at(&location)
            .with_decal_order(0)
            .enter(scene);

        Self {
            size: SizePx::default(),
            measured_size,
            scene_transform,
            name,
        }
    }

    pub fn measured_size(&self) -> SizePx {
        self.measured_size
    }

    pub fn set_layout(&mut self, size: SizePx, layout_transform: Transform) {
        self.size = size;
        let local_center = Point::new(size.width as f64 / 2.0, size.height as f64 / 2.0);
        let scene_transform = layout_transform.to_origin_space(local_center);
        self.scene_transform.update_if_changed(scene_transform);
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
            .to_location()
            .relative_to(&parent_location)
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
        let local_center = Point::new(size.width as f64 / 2.0, size.height as f64 / 2.0);
        let scene_transform = layout_transform.to_origin_space(local_center);
        self.scene_transform.update_if_changed(scene_transform);
    }
}
