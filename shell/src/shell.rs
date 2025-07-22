use std::{
    future::Future,
    sync::Arc,
    time::Duration,
};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use anyhow::{anyhow, bail, Result};
use log::info;
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot,
};
use wgpu::{Instance, Surface, SurfaceTarget};
use winit::{
    application::ApplicationHandler,
    dpi,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    window::{Window, WindowAttributes, WindowId},
};

use crate::ShellWindow;
use massive_animation::{Interpolatable, Interpolation, Tickery, Timeline};

pub async fn run<R: Future<Output = Result<()>> + 'static + Send>(
    application: impl FnOnce(ApplicationContext) -> R + 'static + Send,
) -> Result<()> {
    let event_loop = EventLoop::with_user_event().build()?;

    // Spawn application.

    // Robustness: may use unbounded channels.
    let (event_sender, event_receiver) = tokio::sync::mpsc::unbounded_channel();

    // Proxy for sending events to the event loop from another thread.
    let event_loop_proxy = event_loop.create_proxy();

    let tickery = Arc::new(Tickery::new(Instant::now()));

    let application_context = ApplicationContext {
        event_receiver,
        event_loop_proxy,
        tickery: tickery.clone(),
    };

    let (result_tx, mut result_rx) = oneshot::channel();
    let _application_task = tokio::spawn(async move {
        let r = application(application_context).await;
        // Found no way to retrieve the result via JoinHandle, so a return via a onceshot channel
        // must do.
        result_tx
            .send(Some(r))
            .expect("Internal Error: Failed to set the application result");
    });

    // Event loop

    {
        let mut winit_context = WinitApplicationHandler {
            event_sender,
            tickery,
        };

        info!("Entering event loop");
        event_loop.run_app(&mut winit_context)?;
        info!("Exiting event loop");
    };

    // Check application's result
    if let Ok(Some(r)) = result_rx.try_recv() {
        info!("Application ended with {r:?}");
        r?;
    } else {
        // TODO: This should probably be an error, we want to the application to have full
        // control over the lifetime of the winit event loop.
        info!("Application did not end");
    }

    Ok(())
}

#[must_use]
#[derive(Debug, Copy, Clone)]
pub enum ControlFlow {
    Exit,
    Continue,
}

pub enum ShellRequest {
    CreateWindow {
        // Box because of large size.
        attributes: Box<WindowAttributes>,
        on_created: oneshot::Sender<Result<Window>>,
    },
    /// Surfaces need to be created on the main thread on macOS when a window handle is provided.
    CreateSurface {
        instance: Instance,
        target: SurfaceTarget<'static>,
        on_created: oneshot::Sender<Result<Surface<'static>>>,
    },
}

#[derive(Debug)]
pub enum ShellEvent {
    WindowEvent(WindowId, WindowEvent),
}

impl ShellEvent {
    #[must_use]
    pub fn window_event_for(&self, window: &ShellWindow) -> Option<&WindowEvent> {
        match self {
            ShellEvent::WindowEvent(id, window_event) if *id == window.id() => Some(window_event),
            _ => None,
        }
    }
}

#[allow(unused)]
pub fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let r = f();
    info!("{name}: {:?}", start.elapsed());
    r
}

/// The [`ApplicationContext`] is the connection to the runtinme. It allows the application to poll
/// for events while also forwarding events to the renderer.
///
/// In addition to that it provides an animator that is updated with each event (mostly ticks)
/// coming from the shell.
pub struct ApplicationContext {
    event_receiver: UnboundedReceiver<ShellEvent>,
    event_loop_proxy: EventLoopProxy<ShellRequest>,
    tickery: Arc<Tickery>,
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

struct WinitApplicationHandler {
    event_sender: UnboundedSender<ShellEvent>,
    tickery: Arc<Tickery>,
}

impl ApplicationHandler<ShellRequest> for WinitApplicationHandler {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        // Robustness: As recommended, wait for the resumed event before creating any window.
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ShellRequest) {
        match event {
            ShellRequest::CreateWindow {
                attributes,
                on_created,
            } => {
                let r = event_loop.create_window(*attributes);
                on_created
                    .send(r.map_err(|e| e.into()))
                    .expect("oneshot can send");
            }
            ShellRequest::CreateSurface {
                instance,
                target,
                on_created,
            } => {
                let r = instance.create_surface(target);
                on_created
                    .send(r.map_err(|e| e.into()))
                    .expect("oneshot can send");
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if event != WindowEvent::RedrawRequested {
            info!("{event:?}");
        }

        self.send_event(event_loop, ShellEvent::WindowEvent(window_id, event))
    }
}

impl WinitApplicationHandler {
    fn send_event(&mut self, event_loop: &ActiveEventLoop, shell_event: ShellEvent) {
        if let Err(_e) = self.event_sender.send(shell_event) {
            // Don't log when we are already exiting.
            if !event_loop.exiting() {
                info!("Receiver for events dropped, exiting event loop: {_e:?}");
                event_loop.exit();
            }
        }
    }
}
