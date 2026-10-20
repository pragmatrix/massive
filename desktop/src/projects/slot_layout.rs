//! The animated layout of slot content. Launchers and nested projects share it, so that both
//! appear, slide, and follow a re-parent in the same way.

use std::time::Duration;

use massive_animation::{Animated, AnimationAllocator, AnimationProgress, Interpolation};
use massive_geometry::{SizedTransform, Transform};

const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);

/// The layout is `None` until the first one arrives. The first layout is taken without an
/// animation, so new content appears directly in its slot. Animating it would make the content
/// grow out of the matrix's origin.
#[derive(Debug, Default)]
pub struct SlotLayout {
    layout: Option<Animated<SizedTransform>>,
}

impl SlotLayout {
    pub fn set(&mut self, context: &mut dyn AnimationAllocator, layout: SizedTransform) {
        match &mut self.layout {
            Some(animated) => animated.animate_if_changed_with(
                context,
                layout,
                STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            ),
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
        let origin = previous_to_current * drawn.to_origin_space();
        let transform = origin.to_anchor_space(drawn.rect().center());
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
    use massive_geometry::{Quaternion, Size};

    use super::*;

    /// Re-parented content keeps its on-screen position, also when the new parent is a scaled
    /// nested matrix. Its slide therefore starts where the content was.
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
            parent * layout.layout.unwrap().latest().to_origin_space(),
            previous_parent * drawn.to_origin_space()
        );
    }
}
