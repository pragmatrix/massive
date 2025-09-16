//! A wrapper around a regular Scene that adds animation support.
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use derive_more::Deref;

use massive_animation::{Interpolatable, Interpolation, Tickery, Timeline};

#[derive(Debug, Deref)]
pub struct Scene {
    #[deref]
    inner: massive_scene::Scene,
    pub(crate) tickery: Arc<Tickery>,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    pub fn new() -> Self {
        Self {
            inner: Default::default(),
            tickery: Tickery::new(Instant::now()).into(),
        }
    }

    /// Create a timeline with a starting value.
    pub fn timeline<T: Interpolatable + Send>(&self, value: T) -> Timeline<T> {
        self.tickery.timeline(value)
    }

    /// Create a timeline that is animating from a starting value to a target value.
    pub fn animation<T: Interpolatable + 'static + Send>(
        &self,
        value: T,
        target_value: T,
        duration: Duration,
        interpolation: Interpolation,
    ) -> Timeline<T> {
        let mut timeline = self.tickery.timeline(value);
        timeline.animate_to(target_value, duration, interpolation);
        timeline
    }
}
