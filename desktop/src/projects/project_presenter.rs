use std::{sync::Arc, time::Duration};

use massive_animation::{Animated, Interpolation};
use massive_geometry::{Color, Rect};
use massive_scene::{Handle, IntoVisual, Location, Object, Visual};
use massive_shapes::{Shape, StrokeRect};
use massive_shell::Scene;

use super::LaunchGroupProperties;

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
    hover_rect: Animated<Rect>,
    // Idea: can't we just animate a visual / Handle<Visual>?
    // Performance: This is a visual that _always_ lives inside the renderer, even though it does not contain a single shape when alpha = 0.0
    hover_visual: Handle<Visual>,
}

impl ProjectPresenter {
    pub fn new(location: Handle<Location>, scene: &Scene) -> Self {
        Self {
            location: location.clone(),
            hover_alpha: scene.animated(0.0),
            hover_rect: scene.animated(Rect::ZERO),
            hover_visual: create_hover_shapes(None)
                .into_visual()
                .at(location)
                .enter(scene),
        }
    }

    const HOVER_ANIMATION_DURATION: Duration = Duration::from_millis(500);

    pub fn show_hover_rect(&mut self, rect: Rect) {
        let was_visible = self.hover_alpha.final_value() == 1.0;

        self.hover_alpha.animate_if_changed(
            1.0,
            Self::HOVER_ANIMATION_DURATION,
            Interpolation::CubicOut,
        );

        if was_visible {
            self.hover_rect.animate_if_changed(
                rect,
                Self::HOVER_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        } else {
            self.hover_rect.set_immediately(rect);
        }
    }

    pub fn hide_hover_rect(&mut self) {
        self.hover_alpha
            .animate(0.0, Self::HOVER_ANIMATION_DURATION, Interpolation::CubicOut);
    }

    pub fn apply_animations(&mut self) {
        {
            let alpha = self.hover_alpha.value();
            let rect_alpha = (alpha != 0.0).then(|| (self.hover_rect.value(), alpha));

            // Ergonomics: What something like apply_to_if_changed(&mut self.hover_visual) or so?
            //
            // Performance: Can't be update just the shapes here with apply...
            let visual = create_hover_shapes(rect_alpha)
                .into_visual()
                .at(&self.location)
                .with_depth_bias(5);
            self.hover_visual.update_if_changed(visual);
        }
    }
}

fn create_hover_shapes(rect_alpha: Option<(Rect, f32)>) -> Arc<[Shape]> {
    rect_alpha
        .map(|(r, a)| {
            StrokeRect {
                rect: r,
                stroke: (10., 10.).into(),
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
