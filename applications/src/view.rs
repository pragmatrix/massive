use std::path::PathBuf;

use anyhow::{Result, anyhow};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use massive_scene::SceneChange;
use winit::event::{self, DeviceId};

use crate::{InstanceId, instance_context::InstanceRequest};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
/// Some ideas for roles.
pub enum ViewRole {
    #[default]
    Primary,
    Assistant,
    Notification {
        persistent: bool,
    },
}

#[derive(Debug)]
pub struct View {
    requests: UnboundedSender<(InstanceId, InstanceRequest)>,
    events: UnboundedReceiver<ViewEvent>,
}

impl View {
    pub(crate) fn new(
        requests: UnboundedSender<(InstanceId, InstanceRequest)>,
        receiver: UnboundedReceiver<ViewEvent>,
    ) -> Self {
        Self {
            requests,
            events: receiver,
        }
    }

    pub async fn wait_for_event(&mut self) -> Result<ViewEvent> {
        self.events
            .recv()
            .await
            .ok_or(anyhow!("Internal error: View client vanished unexpectedly"))
    }
}

#[derive(Debug)]
pub enum ViewRequest {
    /// Detail: Empty changes should not be possible. It should create an error. Compared to a
    /// window environment, there is no redraw needed when there are no changes.
    Redraw(Vec<SceneChange>),
    /// Feature: This should probably specify a depth too.
    Resize((u32, u32)),
}

/// The side of a view the shell sees.
#[derive(Debug)]
pub struct ViewClient {
    instance: InstanceId,
    role: ViewRole,
    events: UnboundedSender<ViewEvent>,
}

impl ViewClient {
    pub(crate) fn new(
        instance: InstanceId,
        role: ViewRole,
        events: UnboundedSender<ViewEvent>,
    ) -> Self {
        Self {
            instance,
            role,
            events,
        }
    }
}

/// Most of them are taken from winit::WindowEvent and simplified if appropriate.
#[derive(Debug)]
pub enum ViewEvent {
    Resized(u32, u32),
    CloseRequested,
    DroppedFile(PathBuf),
    HoveredFile(PathBuf),
    HoveredFileCancelled,
    /// Feature: This is probably related to a "level of detail" management.
    Focused(bool),
    KeyboardInput {
        device_id: event::DeviceId,
        event: event::KeyEvent,
        is_synthetic: bool,
    },
    /// Ergonomics: Document when this is sent (only when Focused?), otherwise, an explicit query
    /// needs to be made.
    ModifiersChanged(event::Modifiers),
    Ime(event::Ime),
    CursorMoved {
        device_id: event::DeviceId,
        /// (x,y) coords in pixels relative to the top-left corner of the view. Because the range
        /// of this data is limited by the display area and it may have been transformed by
        /// the OS to implement effects such as cursor acceleration, it should not be used
        /// to implement non-cursor-like interactions such as 3D camera control.
        position: (f64, f64),
    },
    CursorEntered {
        device_id: event::DeviceId,
    },
    CursorLeft {
        device_id: event::DeviceId,
    },
    MouseWheel {
        device_id: event::DeviceId,
        delta: event::MouseScrollDelta,
        phase: event::TouchPhase,
    },
    MouseInput {
        device_id: event::DeviceId,
        state: event::ElementState,
        button: event::MouseButton,
    },
    // Feature: PinchGesture, PanGesture, DoubleTapGesture, RotationGesture, TouchpadPressure,
    // AxisMotion, Touch

    // Detail: ScaleFactorChanged may not be needed. If it happens, the system should take care of it.
    ApplyAnimations,
}
