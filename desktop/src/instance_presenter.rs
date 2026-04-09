use std::time::Duration;

use massive_animation::{Animated, Interpolation};
use massive_applications::ViewCreationInfo;
use massive_geometry::{Color, Point, Quaternion, Rect, RectPx, Transform, Vector3};
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
    pub yaw_animation: Animated<f64>,
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
            let visual = background_shape(Rect::ZERO).at(local_location).enter(scene);

            InstanceBackground {
                transform,
                visual,
                local_rect: Rect::ZERO,
            }
        });

        Self {
            state: InstancePresenterState::WaitingForPrimaryView,
            center_translation_animation: scene.animated(initial_center_translation),
            yaw_animation: scene.animated(0.0),
            background,
        }
    }

    pub fn presents_primary_view(&self) -> bool {
        self.state.view().is_some()
    }

    pub fn set_layout(&mut self, rect: RectPx, center_translation: Vector3, yaw: f64, animate: bool) {

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
                center_translation,
                STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
            self.yaw_animation.animate_if_changed(
                yaw,
                STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        } else {
            self.center_translation_animation.set_immediately(center_translation);
            self.yaw_animation.set_immediately(yaw);
            self.apply_animations();
        }
    }

    pub fn apply_animations(&self) {
        let center_translation = self.center_translation_animation.value();
        let yaw = self.yaw_animation.value();

        if let Some(background) = &self.background {
            let local_center = background.local_rect.center();
            background.transform.update_if_changed(Self::transform_with_local_center(
                center_translation,
                (local_center.x, local_center.y),
                yaw,
            ));
        }

        // Feature: Hiding animation.
        let Some(view) = self.state.view() else {
            return;
        };

        // Correct the view's position around its local center.
        // Since the centering uses i32, we preserve snapping behavior from the layouter.
        let center = view.creation_info.extents.center().to_f64();
        let transform = Self::transform_with_local_center(
            center_translation,
            (center.x, center.y),
            yaw,
        );

        view.creation_info
            .location
            .value()
            .transform
            .update_if_changed(transform);
    }

    pub fn transform(&self, local_center: Point) -> Transform {
        let center_translation = self.center_translation_animation.final_value();
        let yaw = self.yaw_animation.final_value();
        Self::transform_with_local_center(center_translation, (local_center.x, local_center.y), yaw)
    }

    fn transform_with_local_center(
        center_translation: Vector3,
        local_center: (f64, f64),
        yaw: f64,
    ) -> Transform {
        let rotation = Quaternion::from_rotation_y(yaw);
        let local_center = Vector3::new(local_center.0, local_center.1, 0.0);
        let origin_translation = center_translation - rotation * local_center;
        Transform::new(origin_translation, rotation, 1.0)
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
