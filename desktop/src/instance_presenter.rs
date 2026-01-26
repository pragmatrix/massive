use massive_animation::{Animated, Interpolation};
use massive_applications::ViewCreationInfo;
use massive_geometry::{RectPx, SizePx, Vector3};

pub use crate::band_presenter::BandPresenter;

#[derive(Debug)]
pub struct InstancePresenter {
    pub state: InstancePresenterState,
    pub panel_size: SizePx,
    /// The rectangle after layout.
    pub rect: RectPx,
    /// The center of the instance's panel. This is also the point the camera should look at at
    /// rest.
    pub center_animation: Animated<Vector3>,
}

#[derive(Debug)]
pub enum InstancePresenterState {
    /// No view yet, animating in.
    Appearing,
    Presenting {
        view: PrimaryViewPresenter,
    },
    Disappearing {
        view: PrimaryViewPresenter,
    },
}

#[derive(Debug)]
pub struct PrimaryViewPresenter {
    pub view: ViewCreationInfo,
}

impl InstancePresenter {
    pub fn rect(&self) -> RectPx {
        self.rect
    }

    pub fn set_rect(&mut self, rect: RectPx, animate: bool) {
        self.rect = rect;
        let (x, y, z) = rect.center().cast().to_3d().into();
        let translation = (x, y, z).into();

        if animate {
            self.center_animation.animate_if_changed(
                translation,
                BandPresenter::STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        } else {
            self.center_animation.set_immediately(translation);
            self.apply_animations();
        }
    }

    pub fn apply_animations(&self) {
        let view = match &self.state {
            InstancePresenterState::Presenting { view }
            | InstancePresenterState::Disappearing { view } => view,
            InstancePresenterState::Appearing => return,
        };

        // Get the translation for the instance.
        let mut translation = self.center_animation.value();

        // And correct the view's position.
        // Since the centering uses i32, we snap to pixel here (what we want!).
        let center = view.view.extents.center().to_f64();
        translation -= Vector3::new(center.x, center.y, 0.0);

        view.view
            .location
            .value()
            .transform
            .update_if_changed(translation.into());
    }
}
