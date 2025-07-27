use std::{sync::Arc, time::Duration};

use anyhow::{anyhow, bail, Result};
use log::info;
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

    pub render_pacing: RenderPacing,
    /// Was the most recent [`ShellEvent`] [`ShellEvent::ApplyAnimations`]?
    pub apply_animations: bool,
}

#[derive(Debug, PartialEq, Eq, Default)]
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

    /// Waits for a shell event and coordinates the renderer.
    ///
    /// This is cancellation safe.
    pub async fn wait_and_coordinate(
        &mut self,
        renderer: &mut AsyncWindowRenderer,
    ) -> Result<ShellEvent> {
        self.synchronize_render_pacing(renderer)?;

        loop {
            select! {
                event = self.event_receiver.recv() => {
                    let Some(event) = event else {
                        // This means that the shell stopped before the application ended, this should not
                        // happen in normal situations.
                        bail!("Internal Error: Shell shut down, no more events")
                    };

                    if let Some(window_event) = event.window_event_for_id(renderer.id()) {
                        // This handles resizes and redraws.
                        renderer.handle_window_event(window_event)?;
                    }
                    self.apply_animations = false;
                    return Ok(event)
                }

                instant = renderer.wait_for_most_recent_presentation() => {
                    let instant = instant?;
                    if self.render_pacing == RenderPacing::Smooth {
                        self.tickery.prepare_frame(instant);
                        self.apply_animations = true;
                        return Ok(ShellEvent::ApplyAnimations);
                    }
                }
            };
        }
    }

    /// Synchronizes the render pacing suggested by the current state of the tickery with the renderer.
    fn synchronize_render_pacing(&mut self, renderer: &mut AsyncWindowRenderer) -> Result<()> {
        // Look at the Tickery to see if there are animations running and update the `RenderPacing`
        // in the renderer if needed.
        let animations_active = self.tickery.any_users();
        let render_pacing_tickery = if animations_active {
            RenderPacing::Smooth
        } else {
            RenderPacing::Fast
        };
        if render_pacing_tickery == self.render_pacing {
            return Ok(());
        }

        info!("Changing renderer pacing to: {render_pacing_tickery:?}");

        let new_present_mode = match render_pacing_tickery {
            RenderPacing::Fast => wgpu::PresentMode::AutoNoVsync,
            RenderPacing::Smooth => wgpu::PresentMode::AutoVsync,
        };
        renderer.post_msg(RendererMessage::SetPresentMode(new_present_mode))?;

        self.render_pacing = render_pacing_tickery;
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
