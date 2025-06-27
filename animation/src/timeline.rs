use std::{
    cell::{Ref, RefCell},
    rc::Rc,
    time::Duration,
};

use crate::{BlendedAnimation, Coordinator, Interpolatable, Interpolation};

/// A timeline represents a value over time that can be animated.
///
/// Timeline implicitly supports animation blending. New animations added are combined with the
/// trajectory of previous animations.
///
/// ADR: Because we need an Instant at the time of an animation, new animations
/// need to be scheduled and installed at the next tick.
#[derive(Debug)]
pub struct Timeline<T> {
    coordinator: Rc<Coordinator>,

    /// The current value.
    value: RefCell<T>,

    animation: RefCell<BlendedAnimation<T>>,
}

impl<T: Interpolatable> Timeline<T> {
    pub fn new(coordinator: Rc<Coordinator>, value: T) -> Self {
        Self {
            coordinator,
            value: value.into(),
            animation: Default::default(),
        }
    }

    pub fn animate_to(
        &mut self,
        target_value: T,
        duration: Duration,
        interpolation: Interpolation,
    ) {
        let current_time = self.coordinator.current_time();
        let starting_value = self.value.borrow().clone();
        self.animation.borrow_mut().animate_to(
            starting_value,
            current_time,
            target_value,
            duration,
            interpolation,
        );
        if self.is_animating() {
            self.coordinator.notify_active();
        }
    }

    pub fn value(&self) -> Ref<T> {
        let current_time = self.coordinator.current_time();

        if let Some(value) = self.animation.borrow_mut().proceed(current_time) {
            self.value.replace(value);
        }

        if self.is_animating() {
            self.coordinator.notify_active();
        }
        self.value.borrow()
    }

    pub fn is_animating(&self) -> bool {
        self.animation.borrow().is_active()
    }
}
