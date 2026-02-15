use massive_animation::{Animated, Interpolation};
use massive_applications::{ViewCreationInfo, ViewId};
use massive_geometry::{RectPx, SizePx, Vector3};

use crate::{
    band_presenter::BandPresenter,
    navigation::{NavigationNode, container, leaf},
};

#[derive(Debug)]
pub struct InstancePresenter {
    pub state: InstancePresenterState,
    pub panel_size: SizePx,
    /// The rectangle after layout.
    pub rect: RectPx,
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
    pub fn rect(&self) -> RectPx {
        self.rect
    }

    pub fn set_rect(&mut self, rect: RectPx, animate: bool) {
        self.rect = rect;
        let (x, y, z) = rect.center().cast().to_3d().into();
        let translation = (x, y, z).into();

        if animate {
            self.center_translation_animation.animate_if_changed(
                translation,
                BandPresenter::STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        } else {
            self.center_translation_animation.set_immediately(translation);
            self.apply_animations();
        }
    }

    /// This returns a container without an id and contains a leaf with the view id, if there is a
    /// view. This way the instance can be focused / navigated without focusing the view.
    pub fn navigation(&self) -> NavigationNode<'_, ViewId> {
        container(None, || {
            if let Some(view) = self.state.view() {
                [leaf(view.creation_info.id, self.rect().into())].into()
            } else {
                Vec::new()
            }
        })
        .with_rect(self.rect.into())
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
