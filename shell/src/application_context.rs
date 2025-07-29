use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Result};
use log::{error, info};
use tokio::{
    select,
    sync::{mpsc::UnboundedReceiver, oneshot},
};
use winit::{
    dpi,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoopProxy},
    window::WindowAttributes,
};

use massive_animation::{Interpolatable, Interpolation, Tickery, Timeline};

use crate::{
    async_window_renderer::RendererMessage, AsyncWindowRenderer, ShellEvent, ShellRequest,
    ShellWindow,
};

/// The [`ApplicationContext`] is the connection to the runtinme. It allows the application to poll
/// for events while also forwarding events to the renderer.
///
/// In addition to that it provides an animator that is updated with each event (mostly ticks)
/// coming from the shell.
#[derive(Debug)]
pub struct ApplicationContext {
    pub event_receiver: UnboundedReceiver<ShellEvent>,
    pub event_loop_proxy: EventLoopProxy<ShellRequest>,
    pub tickery: Arc<Tickery>,

    /// The current render pacing as seen from the application context. This may not reflect
    /// reality, as it is synchronized with the renderer asynchronously.
    pub render_pacing: RenderPacing,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum RenderPacing {
    #[default]
    // Render as fast as possible to be able to represent input changes.
    Fast,
    // Render a smooth as possible so that animations are synced to the frame rate.
    Smooth,
}

impl ApplicationContext {
    pub async fn new_window(
        &self,
        inner_size: impl Into<dpi::Size>,
        _canvas_id: Option<&str>,
    ) -> Result<ShellWindow> {
        #[cfg(target_arch = "wasm32")]
        assert!(
            _canvas_id.is_none(),
            "Rendering to a canvas isn't support yet"
        );
        let (on_created, when_created) = oneshot::channel();
        let attributes = WindowAttributes::default().with_inner_size(inner_size);
        self.event_loop_proxy
            .send_event(ShellRequest::CreateWindow {
                attributes: attributes.into(),
                on_created,
            })
            .map_err(|e| anyhow!(e.to_string()))?;

        let window = when_created.await??;
        Ok(ShellWindow {
            window: window.into(),
            event_loop_proxy: self.event_loop_proxy.clone(),
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[allow(unused)]
    fn new_window_ev(
        &self,
        event_loop: &ActiveEventLoop,
        inner_size: impl Into<dpi::Size>,
        _canvas_id: Option<&str>,
    ) -> Result<ShellWindow> {
        let window =
            event_loop.create_window(WindowAttributes::default().with_inner_size(inner_size))?;
        Ok(ShellWindow {
            window: Arc::new(window),
            event_loop_proxy: self.event_loop_proxy.clone(),
        })
    }

    #[cfg(target_arch = "wasm32")]
    fn new_window_ev(
        &self,
        event_loop: &ActiveEventLoop,
        // We don't set inner size, the canvas defines how large we render.
        _inner_size: impl Into<dpi::Size>,
        canvas_id: Option<&str>,
    ) -> Result<ShellWindow> {
        use wasm_bindgen::JsCast;
        use winit::platform::web::WindowAttributesExtWebSys;

        let canvas_id = canvas_id.expect("Canvas id is needed for wasm targets");

        let canvas = web_sys::window()
            .expect("No Window")
            .document()
            .expect("No document")
            .query_selector(&format!("#{canvas_id}"))
            // what a shit-show here, why is the error not compatible with anyhow.
            .map_err(|err| anyhow::anyhow!(err.as_string().unwrap()))?
            .expect("No Canvas with a matching id found");

        let canvas: web_sys::HtmlCanvasElement = canvas
            .dyn_into()
            .map_err(|_| anyhow::anyhow!("Failed to cast to HtmlCanvasElement"))?;

        let window =
            event_loop.create_window(WindowAttributes::default().with_canvas(Some(canvas)))?;

        Ok(ShellWindow {
            window: Rc::new(window),
        })
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

    /// Waits for a shell event and updates all timelines if a [`ShellEvent::ApplyAnimations`] is
    /// received.
    pub async fn wait_for_event(&mut self) -> Result<ShellEvent> {
        let event = self.event_receiver.recv().await;
        let Some(event) = event else {
            // This means that the shell stopped before the application ended, this should not
            // happen in normal situations.
            bail!("Internal Error: Shell shut down, no more events")
        };

        // if let ShellEvent::ApplyAnimations(tick) = event {
        //     // Animations may have been removed in the meantime, so we check for wants_ticks()...
        //     self.tickery.tick(tick);
        //     // Even if nothing happened, the event _must_ be forwarded to the application, because
        //     // it may need to apply final values now.
        // }

        Ok(event)
    }

    pub async fn wait_for_shell_event(
        &mut self,
        renderer: &mut AsyncWindowRenderer,
    ) -> Result<ShellEvent> {
        loop {
            select! {
                event = self.event_receiver.recv() => {
                    let Some(event) = event else {
                        // This means that the shell stopped before the application ended, this should not
                        // happen in normal situations.
                        bail!("Internal Error: Shell shut down, no more events")
                    };

                    return Ok(event);
                }

                _instant = renderer.wait_for_most_recent_presentation() => {
                    if self.render_pacing == RenderPacing::Smooth {
                        return Ok(ShellEvent::ApplyAnimations);
                    }
                    // else: Wasn't in a animation cycle: loop and wait for an input event.
                }
            };
        }
    }

    pub fn begin_update_cycle<'a>(
        // Not only do we need &mut self in the Drop handler, but this also prevents users to
        // start a second update cycle in parallel. But this may be allowed?
        // (right now, only .animation is used, which needs only the tickery).
        &'a mut self,
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
                            renderer
                                .post_msg(RendererMessage::Resize((size.width, size.height)))?;
                            UpdateCycleMode::WindowResize
                        }
                        WindowEvent::RedrawRequested => UpdateCycleMode::RedrawRequested,
                        _ => UpdateCycleMode::ExternalOrInteractionEvent,
                    }
                }
                ShellEvent::WindowEvent(_, _) => {
                    bail!("Received an event from a foreign window");
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
            ctx: self,
            renderer,
        })
    }

    fn end_update_cycle(cycle: &mut UpdateCycle) -> Result<()> {
        // Issue a redraw before potentially chaning the render pacing.
        if cycle.mode == UpdateCycleMode::RedrawRequested {
            cycle.renderer.post_msg(RendererMessage::Redraw)?;
        }

        let animations_before = cycle.ctx.render_pacing == RenderPacing::Smooth;
        let animations_detected = cycle.ctx.tickery.animation_ticks_requested();

        match cycle.mode {
            UpdateCycleMode::ExternalOrInteractionEvent
            | UpdateCycleMode::WindowResize
            | UpdateCycleMode::RedrawRequested => {
                // For these cycle modes, we only allow upgrades to the Smooth render pacing
                if !animations_before && animations_detected {
                    info!("Enabling smooth rendering (animations on)");
                    assert_eq!(cycle.ctx.render_pacing, RenderPacing::Fast);
                    cycle
                        .ctx
                        .synchronize_render_pacing(RenderPacing::Smooth, cycle.renderer)?;
                    assert_eq!(cycle.ctx.render_pacing, RenderPacing::Smooth);
                }
            }
            UpdateCycleMode::ApplyAnimations => {
                if animations_before && !animations_detected {
                    info!("Disabling smooth rendering (animations off)");
                    assert_eq!(cycle.ctx.render_pacing, RenderPacing::Smooth);
                    cycle
                        .ctx
                        .synchronize_render_pacing(RenderPacing::Fast, cycle.renderer)?;
                    assert_eq!(cycle.ctx.render_pacing, RenderPacing::Fast);
                }
            }
        }

        Ok(())
    }

    // / Waits for a shell event and coordinates the renderer.
    // /
    // / Everything that is active between the invocation here is called the update cycle.
    // /
    // / There are two kinds of update cycles, a reactive update cycle and an animation cycle.
    // /
    // / An update cycle starts when a input event is returned or this function is cancelled.
    // /
    // / A animation cycle starts when -
    // /
    // / - In the previous update cycle an animation was started or
    // /   the previous update cycle was already an animation cycle.
    // / - The previous frame got presented.
    // /
    // / This is cancellation safe.

    // pub async fn wait_and_coordinate(
    //     &mut self,
    //     renderer: &mut AsyncWindowRenderer,
    // ) -> Result<ShellEvent> {
    //     // This covers both types of update cycles.
    //     let fast_render_pacing = self.tickery.animation_ticks_requested();

    //     self.synchronize_render_pacing(
    //         if fast_render_pacing {
    //             RenderPacing::Fast
    //         } else {
    //             RenderPacing::Smooth
    //         },
    //         renderer,
    //     )?;

    //     // Begin an update cycle in case we got cancelled so that another event uses a more up to date tick.
    //     if self.render_pacing == RenderPacing::Fast {
    //         self.tickery.begin_update_cycle(Instant::now());
    //     }

    //     // What if we get cancelled after a animation cycle?

    //     loop {
    //         select! {
    //             event = self.event_receiver.recv() => {
    //                 let Some(event) = event else {
    //                     // This means that the shell stopped before the application ended, this should not
    //                     // happen in normal situations.
    //                     bail!("Internal Error: Shell shut down, no more events")
    //                 };

    //                 if let Some(window_event) = event.window_event_for_id(renderer.window_id()) {
    //                     // This handles resizes and redraws.
    //                     renderer.handle_window_event(window_event)?;
    //                 }
    //                 self.tickery.begin_update_cycle(Instant::now());
    //                 return Ok(event)
    //             }

    //             instant = renderer.wait_for_most_recent_presentation() => {
    //                 let instant = instant?;
    //                 if self.render_pacing == RenderPacing::Smooth {
    //                     self.tickery.begin_animation_cycle(instant);
    //                     return Ok(ShellEvent::ApplyAnimations);
    //                 }
    //                 // else: Wasn't in a animation cycle: loop and wait for an input event.
    //             }
    //         };
    //     }
    // }

    /// Synchronizes the render pacing suggested by the current state of the tickery with the renderer.
    fn synchronize_render_pacing(
        &mut self,
        pacing: RenderPacing,
        renderer: &mut AsyncWindowRenderer,
    ) -> Result<()> {
        if pacing == self.render_pacing {
            return Ok(());
        }

        info!("Changing renderer pacing to: {pacing:?}");

        let new_present_mode = match pacing {
            RenderPacing::Fast => wgpu::PresentMode::AutoNoVsync,
            RenderPacing::Smooth => wgpu::PresentMode::AutoVsync,
        };
        renderer.post_msg(RendererMessage::SetPresentMode(new_present_mode))?;

        self.render_pacing = pacing;
        Ok(())
    }

    /// Wait for the next event of a specific window and forward it to the renderer if needed.
    pub async fn wait_for_window_event(&mut self, window: &ShellWindow) -> Result<WindowEvent> {
        match self.wait_for_event().await? {
            ShellEvent::WindowEvent(window_id, window_event) if window_id == window.id() => {
                Ok(window_event)
            }
            _ => {
                // TODO: Support this somehow.
                panic!("Received event from another window")
            }
        }
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
    ctx: &'a mut ApplicationContext,
    renderer: &'a mut AsyncWindowRenderer,
}

impl UpdateCycle<'_> {
    /// Create a timeline that is animating from a starting value to a target value.
    pub fn animation<T: Interpolatable + 'static + Send>(
        &self,
        value: T,
        target_value: T,
        duration: Duration,
        interpolation: Interpolation,
    ) -> Timeline<T> {
        self.ctx
            .animation(value, target_value, duration, interpolation)
    }
}

impl Drop for UpdateCycle<'_> {
    fn drop(&mut self) {
        if let Err(e) = ApplicationContext::end_update_cycle(self) {
            error!("Error while ending the update cycle: {e:?}")
        }
    }
}
