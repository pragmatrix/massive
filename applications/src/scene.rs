//! A wrapper around a regular Scene that adds animation support.
use std::time::Duration;

use anyhow::Result;
use derive_more::Deref;

use massive_animation::{Animated, AnimationCoordinator, Interpolatable, Interpolation, TimeScale};

use crate::RenderTarget;

#[derive(Debug, Deref)]
pub struct Scene {
    #[deref]
    inner: massive_scene::Scene,
    animation_coordinator: AnimationCoordinator,
}

impl Scene {
    pub fn new(animation_coordinator: AnimationCoordinator) -> Self {
        Self {
            inner: Default::default(),
            animation_coordinator,
        }
    }

    /// Create an [`Animated`] with an initial value.
    pub fn animated<T: Interpolatable + Send>(&self, value: T) -> Animated<T> {
        self.animation_coordinator.animated(value)
    }

    /// Create a animated value that is animating from a starting value to a target value.
    pub fn animation<T: Interpolatable + 'static + Send>(
        &self,
        value: T,
        target_value: T,
        duration: Duration,
        interpolation: Interpolation,
    ) -> Animated<T> {
        let mut animated = self.animation_coordinator.animated(value);
        animated.animate_to(target_value, duration, interpolation);
        animated
    }

    /// Creates a animated value that can be used to animate other values.
    ///
    /// This tracks durations from one update cycle to the next and provides a way to animate other
    /// values indirectly so that - even when update cycles are not called in regular intervals -
    /// animations are as smooth as possible.
    ///
    /// As long as a TimeScale is asked to scale values, the system stays in "animation mode"
    /// (attempts to re-render every frame) and sends regular  [`ShellEvent::ApplyAnimations`]s.
    pub fn time_scale(&self) -> TimeScale {
        self.animation_coordinator.time_scale()
    }

    // Render all the current scene changes.
    //
    // Pass in the current shell event if you need to handle redraw requests without scene changes
    // and automatic resizing of the renderer.
    pub fn render_to<E>(
        &self,
        render_target: &mut dyn RenderTarget<Event = E>,
        event: Option<E>,
    ) -> Result<()> {
        render_target.render(self.take_changes()?, &self.animation_coordinator, event)
    }
}
