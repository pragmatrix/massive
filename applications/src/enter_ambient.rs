//! Ambient entry: `.enter()` publishes a scene object into the task's installed change queue.
//!
//! Application code runs inside a task whose queue is installed, so the collector never has to be
//! named at the call site (ADR 0008). The [`Enter`] trait dispatches by type instead of forwarding
//! to one generic function: [`UnenteredLocation`] enters as a transform/location pair in a single
//! batch, while an [`Object`](massive_scene::Object) enters as its own handle.
//!
//! The implementations are concrete rather than one blanket `impl<T: Object>`: such a blanket impl
//! would conflict with the [`UnenteredLocation`] impl, because a downstream crate may still add
//! `Object` for that type. A new `Object` type therefore needs `Enter` implemented here, which
//! fails loudly at the call site instead of silently entering nothing.

use massive_scene::{
    Change, Handle, Location, Object, SceneChange, Transform, UnenteredLocation, Visual,
};

use crate::task_context;

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
