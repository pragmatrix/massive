//! The windows that an instance opens itself while the shell hosts it, which only an application
//! with [`Rendering::Instance`](massive_applications::Rendering::Instance) does. The desktop is
//! such an application.

use anyhow::{Result, anyhow};
use tokio::sync::mpsc::WeakUnboundedSender;
use tokio::sync::oneshot;
use winit::dpi::PhysicalSize;
use winit::event_loop::EventLoopProxy;
use winit::window::WindowAttributes;

use massive_applications::{ApplicationMessage, InstanceContext, PresentationId, ViewId};
use massive_geometry::SizePx;

use crate::ShellWindow;
use crate::shell::ShellCommand;

/// Opens windows from an instance's context.
pub trait InstanceWindows {
    /// Creates a new window.
    ///
    /// Async because the window is created on the event loop's thread. Fails unless the shell hosts
    /// the instance and its application draws its own windows.
    fn new_window(
        &self,
        inner_size: impl Into<SizePx>,
    ) -> impl Future<Output = Result<ShellWindow>> + Send;
}

impl InstanceWindows for InstanceContext {
    fn new_window(
        &self,
        inner_size: impl Into<SizePx>,
    ) -> impl Future<Output = Result<ShellWindow>> + Send {
        let windows = self
            .host_windows()
            .and_then(|windows| windows.downcast_ref::<ShellWindows>())
            .cloned();
        let inner_size = inner_size.into();
        async move {
            let windows = windows.ok_or_else(|| {
                anyhow!(
                    "only the shell opens windows, and only for an instance whose application \
                     draws its own windows"
                )
            })?;
            windows.new_window(ViewId::new(), inner_size).await
        }
    }
}

/// The shell's window factory, which the shell puts into the context of an instance that draws its
/// own windows.
#[derive(Debug, Clone)]
pub struct ShellWindows {
    event_loop_proxy: EventLoopProxy<ShellCommand>,
    /// The instance's event queue, which receives the animation ticks of every window's renderer.
    event_sender: WeakUnboundedSender<ApplicationMessage>,
}

impl ShellWindows {
    pub fn new(
        event_loop_proxy: EventLoopProxy<ShellCommand>,
        event_sender: WeakUnboundedSender<ApplicationMessage>,
    ) -> Self {
        Self {
            event_loop_proxy,
            event_sender,
        }
    }

    /// Creates a window whose events arrive under `view_id`.
    pub async fn new_window(&self, view_id: ViewId, inner_size: SizePx) -> Result<ShellWindow> {
        let (on_created, when_created) = oneshot::channel();
        let presentation_id = PresentationId::new();
        let attributes = WindowAttributes::default()
            .with_inner_size(PhysicalSize::new(inner_size.width, inner_size.height));
        self.event_loop_proxy
            .send_event(ShellCommand::CreateWindow {
                view_id,
                attributes: attributes.into(),
                on_created,
            })
            .map_err(|e| anyhow!(e.to_string()))?;

        let window = when_created.await??;
        Ok(ShellWindow::new(
            view_id,
            presentation_id,
            window,
            self.event_loop_proxy.clone(),
            self.event_sender.clone(),
        ))
    }
}
