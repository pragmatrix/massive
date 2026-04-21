use std::{sync::Arc, time::Duration};

use massive_animation::{Animated, Interpolation};
use massive_geometry::{Color, Point, Rect, Transform, Vector3};
use massive_scene::{Handle, IntoVisual, Location, Object, Visual};
use massive_shapes::{Shape, StrokeRect};
use massive_shell::Scene;

use super::LaunchGroupProperties;
use crate::instance_presenter::InstancePresenter;

#[derive(Debug)]
pub struct ProjectPresenter {
    /// The project hierarchy is used for layout. It references the presenters through `GroupIds` and
    /// `LaunchProfileIds`.
    // project: Project,
    pub location: Handle<Location>,

    // groups: HashMap<GroupId, GroupPresenter>,
    // launchers: HashMap<LaunchProfileId, LauncherPresenter>,

    // Idea: Use a type that combines Alpha with another Interpolatable type.
    // Robustness: Alpha should be a type.
    hover_alpha: Animated<f32>,
    hover_rect: Rect,
    hover_transform: Transform,
    hover_scene_transform: Handle<Transform>,
    hover_location: Handle<Location>,
    // Idea: can't we just animate a visual / Handle<Visual>?
    // Performance: This is a visual that _always_ lives inside the renderer, even though it does not contain a single shape when alpha = 0.0
    hover_visual: Handle<Visual>,
}

impl ProjectPresenter {
    const HOVER_STROKE: (f64, f64) = (10.0, 10.0);

    pub fn new(location: Handle<Location>, scene: &Scene) -> Self {
        let hover_scene_transform = Transform::IDENTITY.enter(scene);
        let hover_location =
            Location::new(None, hover_scene_transform.clone()).enter(scene);

        Self {
            location: location.clone(),
            hover_alpha: scene.animated(0.0),
            hover_rect: Rect::ZERO,
            hover_transform: Transform::IDENTITY,
            hover_scene_transform,
            hover_location: hover_location.clone(),
            hover_visual: create_hover_shapes(None)
                .into_visual()
                .at(hover_location)
                .enter(scene),
        }
    }

    const HOVER_ANIMATION_DURATION: Duration = Duration::from_millis(250);

    pub fn set_hover_rect(&mut self, placement: Option<(Rect, Transform)>) {
        match placement {
            Some((rect, transform)) => {
                self.hover_alpha.animate_if_changed(
                    1.0,
                    Self::HOVER_ANIMATION_DURATION,
                    Interpolation::CubicOut,
                );

                self.hover_rect = rect;
                self.hover_transform = transform;
            }
            None => {
                self.hover_alpha.animate_if_changed(
                    0.0,
                    Self::HOVER_ANIMATION_DURATION,
                    Interpolation::CubicOut,
                );
            }
        }
    }

    pub fn apply_animations(&mut self) {
        let alpha = self.hover_alpha.value();
        // Hover shapes are drawn in local coordinates (origin-based rect).
        let local_rect = self.hover_rect.size().to_rect();
        let rect_alpha = (alpha != 0.0).then_some((local_rect, alpha));

        // Position the hover visual in world space. For instances, the layout transform's
        // translate IS the center position (possibly offset for visor layout). For launchers,
        // the transform is IDENTITY so we derive position from the rect's center.
        let local_center = local_rect.center();
        let has_translate = self.hover_transform.translate != Vector3::ZERO;
        let center_transform = if has_translate {
            self.hover_transform
        } else {
            let center = self.hover_rect.center();
            Transform::new(
                Vector3::new(center.x, center.y, 0.0),
                self.hover_transform.rotate,
                self.hover_transform.scale,
            )
        };
        let scene_transform = InstancePresenter::transform_with_layout(
            center_transform,
            Point::new(local_center.x, local_center.y),
        );
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
            let stroke = ProjectPresenter::HOVER_STROKE;
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
pub struct GroupPresenter {
    pub properties: LaunchGroupProperties,
    pub rect: Rect,
}

impl GroupPresenter {
    pub fn new(properties: LaunchGroupProperties) -> Self {
        Self {
            properties,
            rect: Rect::default(),
        }
    }
}
