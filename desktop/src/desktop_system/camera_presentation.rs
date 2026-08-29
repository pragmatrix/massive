use std::time::Instant;

use massive_animation::{Animated, AnimationAllocator, Interpolation};
use massive_geometry::PixelCamera;

use crate::instance_presenter::STRUCTURAL_ANIMATION_DURATION;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraPresentationMode {
    Animate,
    Snap,
    Freeze,
}

impl CameraPresentationMode {
    pub fn permit_camera_moves(self) -> bool {
        match self {
            Self::Animate | Self::Snap => true,
            Self::Freeze => false,
        }
    }
}

#[derive(Debug)]
pub struct CameraPresentation {
    desired: Option<PixelCamera>,
    presented: Animated<PixelCamera>,
}

impl CameraPresentation {
    pub fn new(presented: PixelCamera) -> Self {
        Self {
            desired: None,
            presented: presented.into(),
        }
    }

    pub fn set_desired(&mut self, desired: Option<PixelCamera>) {
        self.desired = desired;
    }

    pub fn synchronize(
        &mut self,
        animation_time: Instant,
        context: &mut dyn AnimationAllocator,
        mode: CameraPresentationMode,
    ) {
        match mode {
            CameraPresentationMode::Animate => {
                if let Some(desired) = self.desired {
                    self.presented.animate_if_changed(
                        context,
                        desired,
                        STRUCTURAL_ANIMATION_DURATION,
                        Interpolation::CubicOut,
                    );
                }
            }
            CameraPresentationMode::Snap => {
                if let Some(desired) = self.desired {
                    self.presented.snap(desired);
                }
            }
            CameraPresentationMode::Freeze => {
                if self.presented.is_animating() {
                    let presented = *self.presented.proceed(animation_time);
                    self.presented.snap(presented);
                }
            }
        }
    }

    pub fn proceed(&mut self, instant: Instant) -> &PixelCamera {
        self.presented.proceed(instant)
    }
}
