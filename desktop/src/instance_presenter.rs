use std::time::Duration;

use massive_animation::{Animated, Interpolation};
use massive_applications::ViewCreationInfo;
use massive_geometry::{Color, Rect, RectPx, Transform, Vector3};
use massive_scene::{At, Handle, Location, Object, ToLocation, Visual};
use massive_shapes::{self as shapes, Shape};
use massive_shell::Scene;

pub const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);
const INSTANCE_BACKGROUND_COLOR: Color = Color::rgb_u32(0x282828);

#[derive(Debug)]
pub struct InstancePresenter {
    pub state: InstancePresenterState,
    /// The center of the instance's panel. This is also the point the camera should look at at
    /// rest.
    pub center_translation_animation: Animated<Vector3>,
    background: Option<InstanceBackground>,
}

#[derive(Debug)]
struct InstanceBackground {
    transform: Handle<Transform>,
    visual: Handle<Visual>,
    local_rect: Rect,
}

#[derive(Debug)]
pub enum InstancePresenterState {
    /// No view yet, animating in.
    WaitingForPrimaryView,
    Presenting {
        view: PrimaryViewPresenter,
    },
    Disappearing,
}

#[derive(Debug)]
pub struct PrimaryViewPresenter {
    pub creation_info: ViewCreationInfo,
}

impl InstancePresenter {
    pub fn new(
        initial_center_translation: Vector3,
        show_background: bool,
        location: Handle<Location>,
        scene: &Scene,
    ) -> Self {
        let background = show_background.then(|| {
            let transform = Transform::IDENTITY.enter(scene);
            let local_location = transform.to_location().relative_to(location).enter(scene);
            let visual = background_shape(Rect::ZERO)
                .at(local_location)
                .with_decal_order(0)
                .enter(scene);

            InstanceBackground {
                transform,
                visual,
                local_rect: Rect::ZERO,
            }
        });

        Self {
            state: InstancePresenterState::WaitingForPrimaryView,
            center_translation_animation: scene.animated(initial_center_translation),
            background,
        }
    }

    pub fn presents_primary_view(&self) -> bool {
        self.state.view().is_some()
    }

    pub fn set_rect(&mut self, rect: RectPx, z_offset: f64, animate: bool) {
        let center = rect.center().cast();
        let translation = (center.x, center.y, z_offset).into();

        if let Some(background) = &mut self.background {
            let rect: Rect = rect.into();
            background.local_rect = rect.size().to_rect();
            background.visual.update_with_if_changed(|visual| {
                let local_rect = background.local_rect;
                visual.shapes = [background_shape(local_rect)].into();
            });
        }

        if animate {
            self.center_translation_animation.animate_if_changed(
                translation,
                STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        } else {
            self.center_translation_animation
                .set_immediately(translation);
            self.apply_animations();
        }
    }

    pub fn apply_animations(&self) {
        let center_translation = self.center_translation_animation.value();

        if let Some(background) = &self.background {
            let local_center = background.local_rect.center();
            let background_origin_x = center_translation.x - local_center.x;
            let background_origin_y = center_translation.y - local_center.y;
            background.transform.update_if_changed(
                (
                    background_origin_x,
                    background_origin_y,
                    center_translation.z,
                )
                    .into(),
            );
        }

        // Feature: Hiding animation.
        let Some(view) = self.state.view() else {
            return;
        };

        // Get the translation for the instance.
        let mut translation = center_translation;

        // And correct the view's position.
        // Since the centering uses i32, we snap to pixel here (what we want!).
        let center = view.creation_info.extents.center().to_f64();
        translation -= Vector3::new(center.x, center.y, 0.0);

        view.creation_info
            .location
            .value()
            .transform
            .update_if_changed(translation.into());
    }
}

impl InstancePresenterState {
    pub fn view(&self) -> Option<&PrimaryViewPresenter> {
        match self {
            Self::WaitingForPrimaryView => None,
            Self::Presenting { view } => Some(view),
            Self::Disappearing => None,
        }
    }
}

fn background_shape(rect: Rect) -> Shape {
    shapes::Rect::new(rect, INSTANCE_BACKGROUND_COLOR).into()
}
