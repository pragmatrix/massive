//! A wrapper around a regular Scene that adds animation support.
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use derive_more::Deref;
use log::{error, info};
use massive_geometry::Camera;
use winit::event::WindowEvent;

use crate::{AsyncWindowRenderer, RenderPacing, ShellEvent};
use massive_animation::{Animated, Interpolatable, Interpolation, Tickery, TimeScale};

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

    /// Create an [`Animated`] with an initial value.
    pub fn animated<T: Interpolatable + Send>(&self, value: T) -> Animated<T> {
        self.tickery.animated(value)
    }

    /// Create a animated value that is animating from a starting value to a target value.
    pub fn animation<T: Interpolatable + 'static + Send>(
        &self,
        value: T,
        target_value: T,
        duration: Duration,
        interpolation: Interpolation,
    ) -> Animated<T> {
        let mut animated = self.tickery.animated(value);
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
        self.tickery.time_scale()
    }

    /// Begin an update cycle.
    ///
    /// The update cycle is used at a time the scene changes and the renderer needs to be informed
    /// at the end of the update cycle about the changes.
    pub fn begin_update_cycle<'a>(
        // Not only do we need &mut self in the Drop handler, but this also prevents users to
        // start a second update cycle in parallel. But this may be allowed?
        // (right now, only .animation is used, which needs only the tickery).
        &'a self,
        renderer: &'a mut AsyncWindowRenderer,
        event: Option<&ShellEvent>,
    ) -> Result<UpdateCycle<'a>> {
        // Handle the window event.
        let mode = if let Some(event) = event {
            match event {
                ShellEvent::WindowEvent(window_id, window_event)
                    if *window_id == renderer.window_id() =>
                {
                    match window_event {
                        WindowEvent::Resized(size) => {
                            // A resize is sent to the renderer first, so that we can prepare it for the right size
                            // as soon as possible.
                            //
                            // Performance: Does a resize block inside the async renderer if there is a pending
                            // presentation?
                            renderer.resize((size.width, size.height))?;
                            UpdateCycleMode::WindowResize
                        }
                        WindowEvent::RedrawRequested => UpdateCycleMode::RedrawRequested,
                        _ => UpdateCycleMode::ExternalOrInteractionEvent,
                    }
                }
                ShellEvent::WindowEvent(_, _) => {
                    bail!("Received an event from another window");
                }

                ShellEvent::ApplyAnimations => {
                    // Optimization: This Instant::now() should not be used for animation cycles,
                    // (Apply Animations could really carry the previous presentation time)
                    UpdateCycleMode::ApplyAnimations
                }
            }
        } else {
            UpdateCycleMode::ExternalOrInteractionEvent
        };

        let apply_animations = mode == UpdateCycleMode::ApplyAnimations;
        self.tickery
            .begin_update_cycle(Instant::now(), apply_animations);

        Ok(UpdateCycle {
            mode,
            scene: self,
            renderer,
        })
    }

    fn end_update_cycle(cycle: &mut UpdateCycle) -> Result<()> {
        // Push scene changes to the renderer.

        let changes = cycle.scene.take_changes()?;
        let any_scene_changes = !changes.is_empty();

        // Push the changes _directly_ to the renderer which picks it up in the next redraw. This
        // may asynchronously overtake the subsequent redraw request if a previous was pending.
        //
        // Architecture: We could send this through the RendererMessage::Redraw, which may cause
        // other problems (increased latency and the need for combining changes if Redraws are
        // pending).
        if any_scene_changes {
            cycle.renderer.change_collector().push_many(changes);
        }

        // Issue a redraw before potentially changing the render pacing.
        if any_scene_changes || cycle.mode == UpdateCycleMode::RedrawRequested {
            cycle.renderer.redraw()?;
        }

        // Update render pacing depending on if there are active animations.

        let animations_before = cycle.renderer.pacing() == RenderPacing::Smooth;
        let animations_now = cycle.scene.tickery.animation_ticks_needed();

        match cycle.mode {
            UpdateCycleMode::ExternalOrInteractionEvent
            | UpdateCycleMode::WindowResize
            | UpdateCycleMode::RedrawRequested => {
                // For these cycle modes, we only allow upgrades to the Smooth render pacing
                if !animations_before && animations_now {
                    info!("Enabling smooth rendering (animations on)");
                    debug_assert_eq!(cycle.renderer.pacing(), RenderPacing::Fast);
                    cycle.renderer.update_render_pacing(RenderPacing::Smooth)?;
                    debug_assert_eq!(cycle.renderer.pacing(), RenderPacing::Smooth);
                }
            }
            UpdateCycleMode::ApplyAnimations => {
                assert!(animations_before);
                if !animations_now {
                    info!("Disabling smooth rendering (animations off)");
                    debug_assert_eq!(cycle.renderer.pacing(), RenderPacing::Smooth);
                    cycle.renderer.update_render_pacing(RenderPacing::Fast)?;
                    debug_assert_eq!(cycle.renderer.pacing(), RenderPacing::Fast);
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
enum UpdateCycleMode {
    ExternalOrInteractionEvent,
    WindowResize,
    RedrawRequested,
    ApplyAnimations,
}

#[derive(Debug)]
pub struct UpdateCycle<'a> {
    mode: UpdateCycleMode,
    /// The scene, so that we can push the changes at the end of the cycle to the renderer.
    scene: &'a Scene,
    renderer: &'a mut AsyncWindowRenderer,
}

impl UpdateCycle<'_> {
    /// Ergonomics: Since Scene can only stage here, what about implementing the stage() function directly on
    /// UpdateCycle?
    pub fn scene(&self) -> &Scene {
        self.scene
    }

    pub fn update_camera(&mut self, camera: Camera) -> Result<()> {
        self.renderer.update_camera(camera)
    }
}

impl Drop for UpdateCycle<'_> {
    fn drop(&mut self) {
        if let Err(e) = Scene::end_update_cycle(self) {
            error!("Error while ending the update cycle: {e:?}")
        }
    }
}
