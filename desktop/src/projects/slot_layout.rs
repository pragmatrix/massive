//! The animated layout of slot content, shared by launchers and nested projects so both appear,
//! slide, and re-base alike.

use std::time::Duration;

use massive_animation::{Animated, AnimationAllocator, AnimationProgress, Interpolation};
use massive_geometry::{SizedTransform, Transform};

const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);

/// `None` until the first layout, which is taken as it is, so new content appears in its slot
/// instead of growing out of its matrix's origin.
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

    /// Moves the drawn layout by `previous_to_current`, the map from the previous parent's space
    /// into the current one, without animating. It reads the drawn value, not the target, so
    /// content re-parented mid-animation continues from where it is. Content re-parented before
    /// its first layout was never drawn, so there is nothing to re-base.
    pub fn rebase(&mut self, previous_to_current: Transform) {
        let Some(layout) = self.layout.as_mut() else {
            return;
        };
        let drawn = *layout.latest();
        let origin = previous_to_current * drawn.to_origin_space();
        let transform = origin.to_anchor_space(drawn.rect().center());
        layout.snap(SizedTransform::new(drawn.size, transform));
    }

    /// The drawn layout, `None` before the first one.
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

    /// Re-parented content keeps its drawn world transform, also when the new parent is a scaled
    /// nested matrix, so its slide starts where it was.
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
