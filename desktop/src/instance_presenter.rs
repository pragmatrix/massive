use std::time::Duration;

use massive_animation::{Animated, Interpolation};
use massive_applications::ViewCreationInfo;
use massive_geometry::{RectPx, Vector3};

pub const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub struct InstancePresenter {
    pub state: InstancePresenterState,
    /// The center of the instance's panel. This is also the point the camera should look at at
    /// rest.
    pub center_translation_animation: Animated<Vector3>,
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
    pub fn presents_primary_view(&self) -> bool {
        self.state.view().is_some()
    }

    pub fn set_rect(&mut self, rect: RectPx, animate: bool) {
        let (x, y, z) = rect.center().cast().to_3d().into();
        let translation = (x, y, z).into();

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
        // Feature: Hiding animation.
        let Some(view) = self.state.view() else {
            return;
        };

        // Get the translation for the instance.
        let mut translation = self.center_translation_animation.value();

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
