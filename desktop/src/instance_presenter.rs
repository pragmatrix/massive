use massive_animation::{Animated, Interpolation};
use massive_applications::ViewCreationInfo;
use massive_geometry::{RectPx, SizePx, Vector3};
use massive_layout::{LayoutInfo, LayoutNode};

pub use crate::desktop_presenter::{DesktopPresenter, LayoutContext};

#[derive(Debug)]
pub struct InstancePresenter {
    pub state: InstancePresenterState,
    pub panel_size: SizePx,
    /// The center of the instance's panel. This is also the point the camera should look at at
    /// rest.
    pub center_animation: Animated<Vector3>,
    pub view: Option<PrimaryViewPresenter>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum InstancePresenterState {
    /// No view yet, or just appearing, animating in.
    Appearing,
    Presenting,
    Disappearing,
}

#[derive(Debug)]
pub struct PrimaryViewPresenter {
    pub view: ViewCreationInfo,
}

impl InstancePresenter {
    pub fn apply_animations(&self) {
        let Some(view) = &self.view else {
            return;
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

impl LayoutNode<LayoutContext> for InstancePresenter {
    type Rect = RectPx;

    fn layout_info(&self, _context: &LayoutContext) -> LayoutInfo<SizePx> {
        self.panel_size.into()
    }

    fn set_rect(&mut self, rect: Self::Rect, context: &mut LayoutContext) {
        let translation = (rect.origin.x as f64, rect.origin.y as f64, 0.0).into();

        if context.animate {
            self.center_animation.animate_if_changed(
                translation,
                DesktopPresenter::STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        } else {
            self.center_animation.set_immediately(translation);
            self.apply_animations();
        }
    }
}
