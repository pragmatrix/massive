//! The animated layout of slot content. Launchers, nested projects, and project matrices share it,
//! so that all of them appear, animate, and follow a re-parent in the same way.

use massive_animation::{Animated, AnimationAllocator, AnimationProgress};
use massive_geometry::{SizedTransform, Transform};

use crate::policy::constants::structural_animation;

/// The animated layout of one slot's content relative to its matrix, or of a matrix relative to
/// its project.
///
/// The first layout is shown without an animation, because there is no earlier layout to start
/// from, and the content would grow out of the matrix's origin.
#[derive(Debug, Default)]
pub struct SlotLayout {
    /// `None` until the first layout arrives, unless the layout was started at another layout.
    layout: Option<Animated<SizedTransform>>,
}

impl SlotLayout {
    /// A layout shown at `layout` before its first layout arrives, so that the first layout
    /// animates from there.
    pub fn starting_at(layout: SizedTransform) -> Self {
        Self {
            layout: Some(layout.into()),
        }
    }

    /// Animates to `layout` from the layout shown now. A running animation is replaced, not blended
    /// with the new one. Nested layouts, such as a slot in its project, then run on the same curve,
    /// so the product of their scales on screen does not swing when a change interrupts another.
    pub fn set(&mut self, context: &mut dyn AnimationAllocator, layout: SizedTransform) {
        match &mut self.layout {
            Some(animated) => {
                if *animated.target() == layout {
                    return;
                }
                let shown = *animated.latest();
                animated.snap(shown);
                animated.animate_with(
                    context,
                    layout,
                    structural_animation::DURATION,
                    structural_animation::INTERPOLATION,
                );
            }
            None => self.layout = Some(layout.into()),
        }
    }

    /// Converts the on-screen layout with `previous_to_current`, which maps the previous parent's
    /// coordinates into the current parent's. Nothing is animated here.
    ///
    /// The conversion starts from the value currently shown, not from the animation's target.
    /// Content that is re-parented in the middle of an animation therefore continues from where
    /// it is. Content that is re-parented before its first layout was never shown, so there is
    /// nothing to convert.
    pub fn rebase(&mut self, previous_to_current: Transform) {
        let Some(layout) = self.layout.as_mut() else {
            return;
        };
        let drawn = *layout.latest();
        let origin = previous_to_current * drawn.pivoted_at_origin();
        let transform = origin.pivoted_at(drawn.rect().center());
        layout.snap(SizedTransform::new(drawn.size, transform));
    }

    /// Returns the layout to show at `progress`. It is `None` before the first layout arrived.
    pub fn proceed_with(&mut self, progress: AnimationProgress) -> Option<SizedTransform> {
        self.layout
            .as_mut()
            .map(|layout| *layout.proceed_with(progress))
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
        let mut layout = SlotLayout {
            layout: Some(drawn.into()),
        };

        layout.rebase(parent.inverse() * previous_parent);

        assert_eq!(
            parent * layout.layout.unwrap().latest().pivoted_at_origin(),
            previous_parent * drawn.pivoted_at_origin()
        );
    }

    /// A change that interrupts a running animation starts a new one from the layout shown at that
    /// moment, instead of blending the two, so that nested layouts stay on the same curve.
    #[test]
    fn an_interrupting_change_replaces_the_running_animation() {
        let start = Instant::now();
        let at = |offset: Duration| move |_: Duration| start + offset;
        let layout_at = |x: f64| {
            SizedTransform::new(
                Size::new(100.0, 100.0),
                Transform::from_translation((x, 0.0, 0.0)),
            )
        };
        let mut layout = SlotLayout::starting_at(layout_at(0.0));
        layout.set(&mut Allocator(at(Duration::ZERO)), layout_at(100.0));
        let halfway = structural_animation::DURATION / 2;
        let shown = layout.proceed_with(AnimationProgress::Proceed(start + halfway));

        layout.set(&mut Allocator(at(halfway)), layout_at(-100.0));

        let animated = layout.layout.as_ref().unwrap();
        assert_eq!(animated.animation_count(), 1);
        assert_eq!(Some(*animated.latest()), shown);
    }

    struct Allocator<F>(F);

    impl<F: FnMut(Duration) -> Instant> AnimationAllocator for Allocator<F> {
        fn allocate_animation_time(&mut self, duration: Duration) -> Instant {
            (self.0)(duration)
        }
    }
}
