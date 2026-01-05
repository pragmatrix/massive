//! A wrapper around a regular Scene that adds animation support.
use std::{
    ops::{Add, Mul},
    time::Duration,
};

use anyhow::Result;
use derive_more::Deref;

use massive_animation::{Animated, AnimationCoordinator, Interpolatable, Interpolation, TimeScale};
use massive_renderer::{RenderPacing, RenderSubmission, RenderTarget};

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
    pub fn animation<
        T: Interpolatable + 'static + Send + Add<Output = T> + Mul<f64, Output = T>,
    >(
        &self,
        value: T,
        target_value: T,
        duration: Duration,
        interpolation: Interpolation,
    ) -> Animated<T> {
        let mut animated = self.animation_coordinator.animated(value);
        animated.animate(target_value, duration, interpolation);
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

    /// Accumulate external changes into this scene.
    pub fn accumulate_changes(&self, changes: massive_scene::SceneChanges) {
        self.inner.push_changes(changes);
    }

    // Render all the current scene changes.
    //
    // Pass in the current shell event if you need to handle redraw requests without scene changes
    // and automatic resizing of the renderer.
    pub fn render_to(&self, render_target: &mut dyn RenderTarget) -> Result<()> {
        render_target.render(self.begin_frame())
    }

    /// Take all changes from the Scene and return a RenderSubmission.
    pub fn begin_frame(&self) -> RenderSubmission {
        let animations_active = self.animation_coordinator.end_cycle();

        let pacing = if animations_active {
            RenderPacing::Smooth
        } else {
            RenderPacing::Fast
        };

        RenderSubmission::new(self.take_changes(), pacing)
    }
}
