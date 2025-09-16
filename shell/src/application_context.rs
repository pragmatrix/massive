use std::collections::VecDeque;

use anyhow::{Result, anyhow, bail};
use tokio::{
    select,
    sync::{
        mpsc::{UnboundedReceiver, error::TryRecvError},
        oneshot,
    },
};
use winit::{dpi, event_loop::EventLoopProxy, window::WindowAttributes};

use crate::{
    AsyncWindowRenderer, RenderPacing, ShellEvent, ShellRequest, ShellWindow, message_filter,
};

/// The [`ApplicationContext`] is the connection to the runtinme. It allows the application to poll
/// for events while also forwarding events to the renderer.
///
/// In addition to that it provides an animator that is updated with each event (mostly ticks)
/// coming from the shell.
#[derive(Debug)]
pub struct ApplicationContext {
    event_receiver: UnboundedReceiver<ShellEvent>,
    // Used for stuff that needs to run on the event loop thread. Like Window creation, for example.
    event_loop_proxy: EventLoopProxy<ShellRequest>,

    /// ADR: currently here, but should probably be an EventLoop query.
    monitor_scale_factor: Option<f64>,

    /// Pending events received but not yet delivered.
    pending_events: VecDeque<ShellEvent>,
}

impl ApplicationContext {
    pub fn new(
        event_receiver: UnboundedReceiver<ShellEvent>,
        event_loop_proxy: EventLoopProxy<ShellRequest>,
        monitor_scale_factor: Option<f64>,
    ) -> Self {
        Self {
            event_receiver,
            event_loop_proxy,
            monitor_scale_factor,
            pending_events: Default::default(),
        }
    }

    pub fn primary_monitor_scale_factor(&self) -> Option<f64> {
        self.monitor_scale_factor
    }

    /// Creates a new window.
    ///
    /// Async because it needs to communicate with the application's main thread on which the window
    /// is actually created.
    pub async fn new_window(
        &self,
        // Ergonomics: Use a massive-geometry size type here.
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
        Ok(ShellWindow::new(window, self.event_loop_proxy.clone()))
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

    /// Wait for the next shell event.
    ///
    /// `renderer` is needed here so that we know when the renderer finished in animation mode and a
    /// [`ShellEvent::ApplyAnimations`] can be produced.
    pub async fn wait_for_shell_event(
        &mut self,
        renderer: &mut AsyncWindowRenderer,
    ) -> Result<ShellEvent> {
        loop {
            // Pull in every event we can get.
            loop {
                match self.event_receiver.try_recv() {
                    Ok(event) => self.pending_events.push_back(event),
                    Err(TryRecvError::Disconnected) => {
                        bail!("Internal Error: Shell shut down, no more events");
                    }
                    Err(TryRecvError::Empty) => {
                        break;
                    }
                }
            }

            // Skip by key. This is to reduce the resizes, redraws and others.
            // Robustness: Going from VecDequeue to Vec and back is a mess.
            {
                let events: Vec<ShellEvent> = message_filter::keep_last_per_key(
                    self.pending_events.iter().cloned().collect(),
                    |ev| ev.skip_key(),
                );

                self.pending_events = events.into_iter().collect();
            }

            // Robustness: If the main application can't process events fast enough, ApplyAnimations will never come.
            if let Some(pending) = self.pending_events.pop_front() {
                return Ok(pending);
            }
            select! {
                event = self.event_receiver.recv() => {
                    let Some(event) = event else {
                        // This means that the shell stopped before the application ended, this should not
                        // happen in normal situations.
                        bail!("Internal Error: Shell shut down, no more events")
                    };
                    self.pending_events.push_back(event);
                }

                _instant = renderer.wait_for_most_recent_presentation() => {
                    if renderer.pacing() == RenderPacing::Smooth {
                        // Robustness: If applyAnimations will come to fast, we probably should push
                        // them into pending_messages.
                        return Ok(ShellEvent::ApplyAnimations);
                    }
                    // else: Wasn't in a animation cycle: loop and wait for an input event.
                }
            };
        }
    }
}
