use std::{sync::Arc, time::Duration};

use massive_animation::{
    Animated, AnimationAllocator, AnimationProgress, Interpolation, Movement, MovementRuntime,
};
use massive_geometry::{Color, Rect, SizePx, SizedTransform, Transform};
use massive_layout::Placement;
use massive_scene::{Handle, IntoVisual, Location, Object, StageIdentityLocation, Visual};
use massive_shapes::{IntoShape, Shape, StrokeRect};
use massive_shell::Scene;

const HOVER_ANIMATION_DURATION: Duration = Duration::from_millis(250);

/// Presents desktop-level visuals and scene anchors.
///
/// Responsibilities:
/// - Provides the shared parent location for launcher and instance presenters.
/// - Presents the hover outline visual.
#[derive(Debug)]
pub struct DesktopPresenter {
    pub location: Handle<Location>,
    hover_movement: Movement<HoverMovement>,
}

impl DesktopPresenter {
    const HOVER_STROKE: (f64, f64) = (10.0, 10.0);

    pub fn new(
        location: Handle<Location>,
        scene: &Scene,
        movement_runtime: &mut MovementRuntime,
    ) -> Self {
        let (hover_scene_transform, hover_location) = scene.stage_identity_location();
        let hover_visual = create_hover_shapes(None)
            .into_visual()
            .at(&hover_location)
            .enter(scene);
        let hover_movement = movement_runtime
            .movement(HoverMovement::default(), move |movement, context| {
                movement.update_hover_placement_and_visual(
                    context,
                    &hover_scene_transform,
                    &hover_location,
                    &hover_visual,
                );
            })
            .mount();

        Self {
            location,
            hover_movement,
        }
    }

    pub fn set_hover_placement(&self, placement: Option<Placement<Transform, 2>>) {
        self.hover_movement.modify(move |movement, context| {
            movement.set_placement(context, placement);
        });
    }
}

#[derive(Debug)]
struct HoverMovement {
    alpha: Animated<f32>,
    layout: Animated<SizedTransform>,
}

impl Default for HoverMovement {
    fn default() -> Self {
        Self {
            alpha: 0.0.into(),
            layout: SizedTransform::default().into(),
        }
    }
}

impl HoverMovement {
    fn update_hover_placement_and_visual(
        &mut self,
        progress: AnimationProgress,
        hover_scene_transform: &Handle<Transform>,
        hover_location: &Handle<Location>,
        hover_visual: &Handle<Visual>,
    ) {
        let alpha = *self.alpha.proceed(progress);
        let layout = *self.layout.proceed(progress);
        let local_rect = layout.rect();
        let rect_alpha = (alpha != 0.0).then_some((local_rect, alpha));
        hover_scene_transform.update_if_changed(layout.to_origin_space());

        let visual = create_hover_shapes(rect_alpha)
            .into_visual()
            .at(hover_location)
            .with_decal_order(5);
        hover_visual.update_if_changed(visual);
    }

    fn set_placement(
        &mut self,
        context: &mut dyn AnimationAllocator,
        placement: Option<Placement<Transform, 2>>,
    ) {
        match placement {
            Some(placement) => {
                let size = placement.rect.size;
                let layout =
                    SizedTransform::from_pixels(SizePx::new(size[0], size[1]), placement.transform);
                self.alpha.animate_if_changed(
                    context,
                    1.0,
                    HOVER_ANIMATION_DURATION,
                    Interpolation::CubicOut,
                );
                if *self.alpha.latest() == 0.0 {
                    self.layout.snap(layout);
                } else {
                    self.layout.animate_if_changed(
                        context,
                        layout,
                        HOVER_ANIMATION_DURATION,
                        Interpolation::CubicOut,
                    );
                }
            }
            None => self.alpha.animate_if_changed(
                context,
                0.0,
                HOVER_ANIMATION_DURATION,
                Interpolation::CubicOut,
            ),
        }
    }
}

fn create_hover_shapes(rect_alpha: Option<(Rect, f32)>) -> Arc<[Shape]> {
    rect_alpha
        .map(|(rect, alpha)| StrokeRect {
            rect: rect.with_outset(DesktopPresenter::HOVER_STROKE),
            stroke: DesktopPresenter::HOVER_STROKE.into(),
            color: Color::rgb_u32(0xff0000).with_alpha(alpha),
        })
        .map(IntoShape::into_shape)
        .into_iter()
        .collect()
}
