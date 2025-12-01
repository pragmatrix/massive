use std::path::PathBuf;

use anyhow::{Context, Result};
use log::error;
use tokio::sync::mpsc::UnboundedSender;

use uuid::Uuid;
use winit::event::{self, DeviceId};
use winit::window::CursorIcon;

use massive_geometry::Identity;
use massive_input::{AggregationEvent, InputEvent};
use massive_scene::{Handle, Location, Matrix, SceneChanges};

use crate::{
    InstanceId, RenderPacing, RenderTarget, Scene, ViewId, instance_context::InstanceCommand,
};

#[derive(Debug)]
pub struct View {
    instance: InstanceId,
    id: ViewId,
    location: Handle<Location>,
    command_sender: UnboundedSender<(InstanceId, InstanceCommand)>,
}

impl Drop for View {
    fn drop(&mut self) {
        if let Err(e) = self
            .command_sender
            .send((self.instance, InstanceCommand::DestroyView(self.id)))
        {
            error!(
                "Failed to send DestroyView command (is the instance command receiver gone?): {e:?}"
            )
        }
    }
}

impl View {
    pub(crate) fn new(
        instance: InstanceId,
        command_sender: UnboundedSender<(InstanceId, InstanceCommand)>,
        role: ViewRole,
        size: (u32, u32),
        scene: &Scene,
    ) -> Result<Self> {
        let id = ViewId(Uuid::new_v4());

        let view_matrix = scene.stage(Matrix::identity());
        let location = scene.stage(Location::new(None, view_matrix));

        command_sender.send((
            instance,
            InstanceCommand::CreateView(ViewCreationInfo {
                id,
                location: location.clone(),
                role,
                size,
            }),
        ))?;

        Ok(Self {
            instance,
            id,
            location,
            command_sender,
        })
    }

    /// The location's matrix.
    pub fn matrix(&self) -> Handle<Matrix> {
        self.location().value().matrix.clone()
    }

    /// A reference to the location that is used to position the view in the parent desktop space.
    pub fn location(&self) -> &Handle<Location> {
        &self.location
    }

    #[allow(unused)]
    fn resize(&mut self, new_size: (u32, u32)) -> Result<()> {
        self.command_sender
            .send((
                self.instance,
                InstanceCommand::View(self.id, ViewCommand::Resize(new_size)),
            ))
            .context("Failed to send a resize request")
    }

    pub fn set_title(&self, title: impl Into<String>) -> Result<()> {
        self.command_sender
            .send((
                self.instance,
                InstanceCommand::View(self.id, ViewCommand::SetTitle(title.into())),
            ))
            .context("Failed to send a set title request")
    }

    pub fn set_cursor(&self, icon: CursorIcon) -> Result<()> {
        self.command_sender
            .send((
                self.instance,
                InstanceCommand::View(self.id, ViewCommand::SetCursor(icon)),
            ))
            .context("Failed to send a set cursor request")
    }
}

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

#[derive(Debug, Clone)]
pub struct ViewCreationInfo {
    pub id: ViewId,
    pub location: Handle<Location>,
    pub role: ViewRole,
    pub size: (u32, u32),
}

#[derive(Debug)]
pub enum ViewCommand {
    /// Detail: Empty changes are possible because animations active might change.
    Render {
        changes: SceneChanges,
        pacing: RenderPacing,
    },
    /// Feature: This should probably specify a depth too.
    Resize((u32, u32)),
    /// Set the title of the view. The desktop decides how to display it.
    SetTitle(String),
    /// Set the cursor icon for the view.
    SetCursor(CursorIcon),
}

impl RenderTarget for View {
    fn render(&mut self, changes: SceneChanges, pacing: RenderPacing) -> Result<()> {
        self.command_sender
            .send((
                self.instance,
                InstanceCommand::View(self.id, ViewCommand::Render { changes, pacing }),
            ))
            .context("Failed to send a redraw request")
    }
}

/// The events a view can receive.
///
/// Most of them are taken from winit::WindowEvent and simplified if appropriate.
#[derive(Debug, Clone)]
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

    // Detail: ScaleFactorChanged may not be needed. If it happens, the instance manager should take
    // care of it.
}

impl InputEvent for ViewEvent {
    type ScopeId = ViewId;

    fn to_aggregation_event(&self) -> Option<AggregationEvent> {
        match self {
            ViewEvent::CursorMoved {
                device_id,
                position,
            } => Some(AggregationEvent::CursorMoved {
                device_id: *device_id,
                position: (*position).into(),
            }),
            ViewEvent::CursorEntered { device_id } => Some(AggregationEvent::CursorEntered {
                device_id: *device_id,
            }),
            ViewEvent::CursorLeft { device_id } => Some(AggregationEvent::CursorLeft {
                device_id: *device_id,
            }),
            ViewEvent::MouseInput {
                device_id,
                state,
                button,
                ..
            } => Some(AggregationEvent::MouseInput {
                device_id: *device_id,
                state: *state,
                button: *button,
            }),
            ViewEvent::ModifiersChanged(modifiers) => {
                Some(AggregationEvent::ModifiersChanged(*modifiers))
            }
            _ => None,
        }
    }

    fn device(&self) -> Option<DeviceId> {
        match self {
            ViewEvent::KeyboardInput { device_id, .. }
            | ViewEvent::CursorMoved { device_id, .. }
            | ViewEvent::CursorEntered { device_id }
            | ViewEvent::CursorLeft { device_id }
            | ViewEvent::MouseWheel { device_id, .. }
            | ViewEvent::MouseInput { device_id, .. } => Some(*device_id),
            _ => None,
        }
    }
}
