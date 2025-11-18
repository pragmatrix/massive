//! A wrapper around a regular Scene that adds animation support.
use std::time::Duration;

use anyhow::Result;
use derive_more::Deref;
use log::info;
use winit::event::WindowEvent;

use crate::{AsyncWindowRenderer, RenderPacing, ShellEvent};
use massive_animation::{Animated, AnimationCoordinator, Interpolatable, Interpolation, TimeScale};

#[derive(Debug, Deref)]
pub struct Scene {
    #[deref]
    inner: massive_scene::Scene,
    animation_coordinator: AnimationCoordinator,
}

impl Scene {
    pub(crate) fn new(animation_coordinator: AnimationCoordinator) -> Self {
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
    pub fn render_to(
        &self,
        renderer: &mut AsyncWindowRenderer,
        event: Option<ShellEvent>,
    ) -> Result<()> {
        let animations_active = self.animation_coordinator.end_cycle();
        let changes = self.take_changes()?;
        let mut redraw = false;

        // Push the changes _directly_ to the renderer which picks it up in the next redraw. This
        // may asynchronously overtake the subsequent redraw / resize requests if a previous one is
        // currently on its way.
        //
        // Architecture: We could send this through the RendererMessage::Redraw, which may cause
        // other problems (increased latency and the need for combining changes if Redraws are
        // pending).
        //
        // Robustness: This should probably threaded through the redraw pipeline?
        if !changes.is_empty() {
            renderer.change_collector().push_many(changes);
            redraw = true;
        }

        let window_id = renderer.window_id();
        let mut resize = None;
        let mut animations_applied = false;
        match event {
            Some(ShellEvent::WindowEvent(id, window_event)) if id == window_id => {
                match window_event {
                    WindowEvent::RedrawRequested => {
                        redraw = true;
                    }
                    WindowEvent::Resized(size) => {
                        resize = Some((size.width, size.height));
                        // Robustness: Is this needed. Doesn't the system always send a redraw
                        // anyway after each resize?
                        redraw = true
                    }
                    _ => {}
                }
            }
            Some(ShellEvent::ApplyAnimations) => {
                // Even if nothing changed in apply animations, we have to redraw to get a new presentation timestamp.
                redraw = true;
                animations_applied = true;
            }
            _ => {}
        };

        let animations_before = renderer.pacing() == RenderPacing::Smooth;

        let new_render_pacing = match (animations_before, animations_active, animations_applied) {
            (false, true, _) => {
                // Changing from Fast to Smooth requires presentation timestamps to follow. So redraw.
                redraw = true;
                Some(RenderPacing::Smooth)
            }
            // Detail: Changing from Smooth to fast is only possible in response to
            // ApplyAnimations: Only then we know that animations are actually applied to the
            // scene and pushed to the renderer with this update.
            (true, false, true) => Some(RenderPacing::Fast),
            _ => None,
        };

        //
        // Sync with the renderer.
        //

        // Resize first and follow up with a complete redraw.

        if let Some(new_size) = resize {
            renderer.resize(new_size)?;
        }

        // Update render pacing before a redraw:
        // - from instant to smooth: We force a redraw _afterwards_ to get VSync based presentation
        //   timestamps and cause ApplyAnimations.
        // - from smooth to instant: Redraw only when something changed afterwards, but instantly
        //   without VSync.
        if let Some(new_render_pacing) = new_render_pacing {
            info!("Changing render pacing to: {new_render_pacing:?}");
            renderer.update_render_pacing(new_render_pacing)?;
        }

        if redraw {
            renderer.redraw()?;
        }

        Ok(())
    }
}
