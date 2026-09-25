//! Ambient access: scene entry, shaping, and animation resolve the task's installed contexts.
//!
//! Application code runs inside a task whose change queue, shaping context, and animation clock
//! are installed, so none of them has to be named at the call site (ADR 0008). [`Enter`]
//! publishes a scene value into the installed queue; [`AmbientShape`] shapes text with the
//! installed shaper; [`AmbientAnimation`] starts animations on the installed animation clock.
//!
//! [`Enter`] dispatches by type instead of forwarding to one generic function:
//! [`UnenteredLocation`] enters as a transform/location pair in a single batch, while an
//! [`Object`](massive_scene::Object) enters as its own handle. The implementations are concrete
//! rather than one blanket `impl<T: Object>`: such a blanket impl would conflict with the
//! [`UnenteredLocation`] impl, because a downstream crate may still add `Object` for that type. A
//! new `Object` type therefore needs `Enter` implemented here, which fails loudly at the call site
//! instead of silently entering nothing.
//!
//! `SizedTextShaper::shape_with` keeps taking an explicit shaper for callers that hold one already
//! (tests, benchmarks, and code shaping outside a task context); [`AmbientShape::shape`] is the
//! same operation for code that runs inside a task. Likewise [`Animated::animate_with`] takes an
//! explicit allocator, and [`AmbientAnimation`] allocates through the task's frame instead. The
//! traits live in this crate rather than in `massive-shapes`/`massive-animation` because the
//! task-locals they read are installed above them.

use massive_animation::{Animated, Interpolatable, Interpolation};
use massive_scene::{
    Change, Handle, Location, Object, SceneChange, Transform, UnenteredLocation, Visual,
};
use massive_shapes::{GlyphRun, SizedTextShaper};
use std::time::Duration;

use crate::task_context;

/// Shape the first line of the text at its font size with the current task's shaper.
///
/// Implemented for [`SizedTextShaper`] so it reads as the last step of the sizing chain:
/// `label.size(FONT_SIZE).shape()`.
pub trait AmbientShape {
    fn shape(self) -> Option<GlyphRun>;
}

impl AmbientShape for SizedTextShaper<'_> {
    fn shape(self) -> Option<GlyphRun> {
        task_context::with_shaper(|shaping_context| {
            let mut shaper = shaping_context.shaper();
            SizedTextShaper::shape_with(self, &mut shaper)
        })
    }
}

/// Enter a scene value into the current task's change queue.
///
/// The associated [`Entered`](Enter::Entered) type is the handle the caller receives: a location
/// yields its transform and location handles together, everything else yields its own handle.
pub trait Enter: Sized {
    type Entered;

    /// Enter `self` into the current task's change queue.
    fn enter(self) -> Self::Entered;
}

impl Enter for UnenteredLocation {
    type Entered = (Handle<Transform>, Handle<Location>);

    /// A location is entered together with its transform, in one batch.
    fn enter(self) -> Self::Entered {
        task_context::with_changes(|collector| self.enter_in(collector))
    }
}

impl Enter for Visual {
    type Entered = Handle<Self>;

    fn enter(self) -> Self::Entered {
        enter_object(self)
    }
}

impl Enter for Location {
    type Entered = Handle<Self>;

    fn enter(self) -> Self::Entered {
        enter_object(self)
    }
}

impl Enter for Transform {
    type Entered = Handle<Self>;

    fn enter(self) -> Self::Entered {
        enter_object(self)
    }
}

/// Enter one object into the current task's change queue.
fn enter_object<T>(value: T) -> Handle<T>
where
    T: Object + 'static,
    SceneChange: From<Change<T::Change>>,
{
    task_context::with_changes(|collector| massive_scene::enter(collector, value))
}

/// Advance animations on the current task's animation clock.
///
/// The ambient twin of the animation advance: the timestamp comes from the task's frame instead
/// of a parameter. The inherent [`Animated`] methods keep the explicit `*_with` forms for callers
/// that already hold an allocator or timestamp: movement closures receive the movement's own
/// allocator, which records the movement's ending time, and tests drive animations by hand.
pub trait AmbientAnimation<T> {
    /// Animate the value to `target` over `duration`, allocating the start time from the task's
    /// frame.
    fn animate(&mut self, target: T, duration: Duration, interpolation: Interpolation);

    /// [`animate`](Self::animate), but only when the target differs from the animation's current
    /// target.
    fn animate_if_changed(&mut self, target: T, duration: Duration, interpolation: Interpolation);

    /// Advance the animation to the task's current animation time and read the value.
    fn proceed(&mut self) -> &T;
}

impl<T> AmbientAnimation<T> for Animated<T>
where
    T: Send + Interpolatable + PartialEq + 'static,
{
    fn animate(&mut self, target: T, duration: Duration, interpolation: Interpolation) {
        // Allocate the start time on the task's clock and hand the explicit instant to the
        // inherent method: the ambient path needs no allocator object.
        let instant = task_context::allocate_animation_time(duration);
        Animated::animate_at(self, instant, target, duration, interpolation);
    }

    fn animate_if_changed(&mut self, target: T, duration: Duration, interpolation: Interpolation) {
        // Compares against the animation's current target — not the latest value — matching
        // `animate_if_changed_with`, so re-issuing an animation toward its running target is
        // skipped.
        if *self.target() == target {
            return;
        }
        self.animate(target, duration, interpolation);
    }

    fn proceed(&mut self) -> &T {
        Animated::proceed_with(self, task_context::animation_time())
    }
}
