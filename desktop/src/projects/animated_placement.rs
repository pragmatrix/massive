//! The animated placement of slot content. Launchers, nested projects, and project matrices share
//! it, so that all of them appear, animate, and follow a re-parent in the same way.

use massive_animation::{Animated, AnimationAllocator, AnimationProgress};
use massive_geometry::{SizedTransform, Transform};

use crate::policy::constants::structural_animation;

/// The animated placement of one slot's content relative to its matrix, or of a matrix relative to
/// its project.
///
/// The first placement is shown without an animation, because there is no earlier placement to
/// start from, and the content would grow out of the matrix's origin.
#[derive(Debug, Default)]
pub struct AnimatedPlacement {
    /// `None` until the first placement arrives, unless it was started at another one.
    placement: Option<Animated<SizedTransform>>,
}

impl AnimatedPlacement {
    /// Shows `from` before the first placement arrives, so that the first placement animates from
    /// there.
    pub fn animating_in_from(from: SizedTransform) -> Self {
        Self {
            placement: Some(from.into()),
        }
    }

    /// Shows `from` and animates from there to the placement it already has, if any.
    ///
    /// A new group's matrix can get its first placement before its project sets the matrix's start,
    /// so the placement it already has is animated to from the start as well.
    pub fn restart_from(&mut self, context: &mut dyn AnimationAllocator, from: SizedTransform) {
        let placed = self.target();
        *self = Self::animating_in_from(from);
        if let Some(placed) = placed {
            self.set(context, placed);
        }
    }

    /// Animates to `placement` from the placement shown now, replacing a running animation.
    ///
    /// Nested placements then stay on the same curve when a change interrupts another (ADR 0048,
    /// `an_interrupting_change_replaces_the_running_animation`).
    pub fn set(&mut self, context: &mut dyn AnimationAllocator, placement: SizedTransform) {
        match &mut self.placement {
            Some(animated) => {
                if *animated.target() == placement {
                    return;
                }
                let shown = *animated.latest();
                animated.snap(shown);
                animated.animate_with(
                    context,
                    placement,
                    structural_animation::DURATION,
                    structural_animation::INTERPOLATION,
                );
            }
            None => self.placement = Some(placement.into()),
        }
    }

    /// Converts the on-screen placement with `previous_to_current`, which maps the previous
    /// parent's coordinates into the current parent's. Nothing is animated here.
    ///
    /// The conversion starts from the value currently shown, not from the animation's target.
    /// Content that is re-parented in the middle of an animation therefore continues from where
    /// it is. Content that is re-parented before its first placement was never shown, so there is
    /// nothing to convert.
    pub fn rebase(&mut self, previous_to_current: Transform) {
        let Some(placement) = self.placement.as_mut() else {
            return;
        };
        let drawn = *placement.latest();
        let origin = previous_to_current * drawn.pivoted_at_origin();
        let transform = origin.pivoted_at(drawn.rect().center());
        placement.snap(SizedTransform::new(drawn.size, transform));
    }

    /// The placement it animates to, or shows when it does not animate. It is `None` before the
    /// first placement arrived, unless the placement was started at another one.
    pub fn target(&self) -> Option<SizedTransform> {
        self.placement.as_ref().map(|placement| *placement.target())
    }

    /// The placement shown now. It is `None` before the first placement arrived.
    pub fn presented(&self) -> Option<SizedTransform> {
        self.placement.as_ref().map(|placement| *placement.latest())
    }

    /// Returns the placement to show at `progress`. It is `None` before the first placement
    /// arrived.
    pub fn proceed_with(&mut self, progress: AnimationProgress) -> Option<SizedTransform> {
        self.placement
            .as_mut()
            .map(|placement| *placement.proceed_with(progress))
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use massive_geometry::{Quaternion, Size};

    use super::*;

    /// Re-parented content keeps its on-screen position, also when the new parent is a scaled
    /// nested matrix. Its animation therefore starts where the content was.
    #[test]
    fn rebase_keeps_the_drawn_world_transform() {
        let previous_parent = Transform::from_translation((100.0, 0.0, 0.0));
        let parent = Transform::new((300.0, 50.0, 0.0), Quaternion::IDENTITY, 0.5);
        let drawn = SizedTransform::new(
            Size::new(200.0, 100.0),
            Transform::from_translation((10.0, 20.0, 0.0)),
        );
        let mut placement = AnimatedPlacement {
            placement: Some(drawn.into()),
        };

        placement.rebase(parent.inverse() * previous_parent);

        assert_eq!(
            parent * placement.placement.unwrap().latest().pivoted_at_origin(),
            previous_parent * drawn.pivoted_at_origin()
        );
    }

    /// A change that interrupts a running animation starts a new one from the placement shown at
    /// that moment, instead of blending the two, so that nested placements stay on the same curve.
    #[test]
    fn an_interrupting_change_replaces_the_running_animation() {
        let start = Instant::now();
        let at = |offset: Duration| move |_: Duration| start + offset;
        let placement_at = |x: f64| {
            SizedTransform::new(
                Size::new(100.0, 100.0),
                Transform::from_translation((x, 0.0, 0.0)),
            )
        };
        let mut placement = AnimatedPlacement::animating_in_from(placement_at(0.0));
        placement.set(&mut Allocator(at(Duration::ZERO)), placement_at(100.0));
        let halfway = structural_animation::DURATION / 2;
        let shown = placement.proceed_with(AnimationProgress::Proceed(start + halfway));

        placement.set(&mut Allocator(at(halfway)), placement_at(-100.0));

        let animated = placement.placement.as_ref().unwrap();
        assert_eq!(animated.animation_count(), 1);
        assert_eq!(Some(*animated.latest()), shown);
    }

    /// An animated placement that is restarted animates from the start to the placement it already
    /// has, so a new group's matrix does not jump when it is placed before its project.
    #[test]
    fn a_restart_animates_to_the_existing_placement() {
        let start = Instant::now();
        let placement_at = |x: f64| {
            SizedTransform::new(
                Size::new(100.0, 100.0),
                Transform::from_translation((x, 0.0, 0.0)),
            )
        };
        let mut placement = AnimatedPlacement::default();
        placement.set(&mut Allocator(|_| start), placement_at(100.0));

        placement.restart_from(&mut Allocator(|_| start), placement_at(0.0));

        let animated = placement.placement.as_ref().unwrap();
        assert_eq!(*animated.latest(), placement_at(0.0));
        assert_eq!(*animated.target(), placement_at(100.0));
        assert_eq!(animated.animation_count(), 1);
    }

    struct Allocator<F>(F);

    impl<F: FnMut(Duration) -> Instant> AnimationAllocator for Allocator<F> {
        fn allocate_animation_time(&mut self, duration: Duration) -> Instant {
            (self.0)(duration)
        }
    }
}
