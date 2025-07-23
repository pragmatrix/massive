use std::{sync::Arc, time::Duration};

use anyhow::{anyhow, bail, Result};
use tokio::sync::{mpsc::UnboundedReceiver, oneshot};
use winit::{
    dpi,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoopProxy},
    window::WindowAttributes,
};

use massive_animation::{Interpolatable, Interpolation, Tickery, Timeline};

use crate::{AsyncWindowRenderer, ShellEvent, ShellRequest, ShellWindow};

/// The [`ApplicationContext`] is the connection to the runtinme. It allows the application to poll
/// for events while also forwarding events to the renderer.
///
/// In addition to that it provides an animator that is updated with each event (mostly ticks)
/// coming from the shell.
pub struct ApplicationContext {
    pub(crate) event_receiver: UnboundedReceiver<ShellEvent>,
    pub(crate) event_loop_proxy: EventLoopProxy<ShellRequest>,
    pub(crate) tickery: Arc<Tickery>,
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
        let event = self.event_receiver.recv().await;
        let Some(event) = event else {
            // This means that the shell stopped before the application ended, this should not
            // happen in normal situations.
            bail!("Internal Error: Shell shut down, no more events")
        };

        if let Some(window_event) = event.window_event_for_id(renderer.id()) {
            renderer.handle_window_event(window_event)?;
        }

        Ok(event)
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
