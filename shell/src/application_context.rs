use std::collections::VecDeque;

use anyhow::{Result, anyhow, bail};
use tokio::{
    select,
    sync::{
        mpsc::{UnboundedReceiver, UnboundedSender, error::TryRecvError, unbounded_channel},
        oneshot,
    },
};
use winit::{dpi, event_loop::EventLoopProxy, window::WindowAttributes};

use crate::{PresentationTimestamp, ShellEvent, ShellWindow, message_filter, shell::ShellRequest};

/// The [`ApplicationContext`] is the connection to the runtime. It allows the application to poll
/// for events while also forwarding events to the renderer.
///
/// In addition to that it provides an animator that is updated with each event (mostly ticks)
/// coming from the shell.
#[derive(Debug)]
pub struct ApplicationContext {
    event_receiver: UnboundedReceiver<ShellEvent>,
    // Used for stuff that needs to run on the event loop thread. Like Window creation, for example.
    pub(crate) event_loop_proxy: EventLoopProxy<ShellRequest>,

    // Robustness: Should probably an event loop query. May be different for different windows and
    // or when a window is moved?
    monitor_scale_factor: f64,

    /// Pending events received but not yet delivered.
    pending_events: VecDeque<ShellEvent>,

    /// ADR: Decided to collect all presentation timestamps globally, so that we don't have to pass
    /// a renderer to the `wait_for_shell_event` function.
    presentation_timestamps_receiver: UnboundedReceiver<PresentationTimestamp>,
    presentation_timestamps_sender: UnboundedSender<PresentationTimestamp>,
}

impl ApplicationContext {
    pub(crate) fn new(
        event_receiver: UnboundedReceiver<ShellEvent>,
        event_loop_proxy: EventLoopProxy<ShellRequest>,
        monitor_scale_factor: f64,
    ) -> Self {
        let (presentation_timestamps_sender, presentation_timestamps_receiver) =
            unbounded_channel();
        Self {
            event_receiver,
            event_loop_proxy,
            monitor_scale_factor,
            pending_events: Default::default(),
            presentation_timestamps_receiver,
            presentation_timestamps_sender,
        }
    }

    pub fn primary_monitor_scale_factor(&self) -> f64 {
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
        Ok(ShellWindow::new(
            window,
            self.event_loop_proxy.clone(),
            self.presentation_timestamps_sender.clone(),
        ))
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
    pub async fn wait_for_shell_event(&mut self) -> Result<ShellEvent> {
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

                _instant = Self::wait_for_most_recent_presentation(&mut self.presentation_timestamps_receiver) => {
                    // Robustness: If applyAnimations will come to fast, we probably should push
                    // them into pending_events.
                    //
                    // Robustness: The render pacing (as seen from the client or the renderer)
                    // may not currently match Smooth pacing when the event is processed. Not
                    // sure what can be done about this yet and even if it's a problem.
                    //
                    // Feature: Does it make sense to forward the timestamp itself or the window id?
                    return Ok(ShellEvent::ApplyAnimations);
                }
                // else: Not in a animation cycle: loop and wait for an input event.
            };
        }
    }

    /// Wait for the most recent presentation.
    ///
    /// If multiple presentation Instants are available, the most recent one is returned.
    ///
    /// This is cancel safe.
    ///
    /// Robustness: Filter by WindowId
    /// Architecture: May use watch channel for this?
    async fn wait_for_most_recent_presentation(
        receiver: &mut UnboundedReceiver<PresentationTimestamp>,
    ) -> Result<PresentationTimestamp> {
        let most_recent = receiver.recv().await;
        let Some(mut most_recent) = most_recent else {
            bail!("Presentation sender vanished (thread got terminated?)");
        };

        // Get the most recent one available.
        loop {
            match receiver.try_recv() {
                Ok(instant) => most_recent = instant,
                Err(TryRecvError::Empty) => return Ok(most_recent),
                Err(TryRecvError::Disconnected) => {
                    // This should never happen I guess, since we keep one sender here all the time.
                    bail!("Presentation sender vanished!");
                }
            }
        }
    }
}
