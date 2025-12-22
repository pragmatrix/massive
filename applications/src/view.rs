use std::path::PathBuf;

use anyhow::{Context, Result};
use derive_more::{From, Into};
use log::debug;
use tokio::sync::mpsc::{UnboundedSender, error::SendError};

use uuid::Uuid;
use winit::{
    event::{self, DeviceId},
    window::CursorIcon,
};

use massive_geometry::{BoxPx, SizePx};
use massive_input::{AggregationEvent, InputEvent};
use massive_scene::{Handle, Location, SceneChanges, Transform};

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
        if let Err(SendError { .. }) = self
            .command_sender
            .send((self.instance, InstanceCommand::DestroyView(self.id)))
        {
            debug!("Ignored DestroyView command because the command receiver is gone")
        }
    }
}

impl View {
    pub(crate) fn new(
        instance: InstanceId,
        command_sender: UnboundedSender<(InstanceId, InstanceCommand)>,
        role: ViewRole,
        extents: BoxPx,
        scene: &Scene,
    ) -> Result<Self> {
        let id = ViewId(Uuid::new_v4());

        // The parent transform and location to send to the desktop so that it can freely position
        // this view.
        //
        // This is to separate the positioning between this view and the desktop.
        //
        // Detail: This could be done also in the desktop, but for now we want to keep the local
        // location here, so that the desktop can't modify it.
        //
        // Detail: The identity transform here is incorrect but will be adjusted by the desktop
        // based on extents.
        let desktop_transform = scene.stage(Transform::IDENTITY);
        let desktop_location = scene.stage(Location::new(None, desktop_transform));

        // The local transform is the basic center transform.
        //
        // Architecture: Do we need a local location anymore, if it does not make sense for the view
        // to modify it now that a full extents can be provided?
        let local_transform = scene.stage(Transform::IDENTITY);
        let location = scene.stage(Location::new(
            Some(desktop_location.clone()),
            local_transform,
        ));

        command_sender.send((
            instance,
            InstanceCommand::CreateView(ViewCreationInfo {
                id,
                location: desktop_location.clone(),
                role,
                extents,
            }),
        ))?;

        Ok(Self {
            instance,
            id,
            location,
            command_sender,
        })
    }

    /// The location's transform.
    ///
    /// This should not be modified
    // Architecture: Introduce a kind of Immutable handle or read only Handle.
    pub fn transform(&self) -> Handle<Transform> {
        self.location().value().transform.clone()
    }

    /// A reference to the location that is used to position the view in the parent desktop space.
    ///
    /// This should not be modified.
    pub fn location(&self) -> &Handle<Location> {
        &self.location
    }

    #[allow(unused)]
    fn resize(&mut self, new_extents: impl Into<ViewExtent>) -> Result<()> {
        self.command_sender
            .send((
                self.instance,
                InstanceCommand::View(self.id, ViewCommand::Resize(new_extents.into().into())),
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
    pub extents: BoxPx,
}

impl ViewCreationInfo {
    pub fn size(&self) -> SizePx {
        self.extents.size().cast()
    }
}

#[derive(Debug)]
pub enum ViewCommand {
    /// Detail: Empty changes are possible because animations active might change.
    Render {
        changes: SceneChanges,
        pacing: RenderPacing,
    },
    /// Feature: This should probably specify a depth too.
    Resize(BoxPx),
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
    Resized(SizePx),
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

#[derive(Debug, From, Into)]
pub struct ViewExtent(BoxPx);

impl From<SizePx> for ViewExtent {
    fn from(value: SizePx) -> Self {
        Self(BoxPx::from_size(value.to_i32()))
    }
}

impl From<(u32, u32)> for ViewExtent {
    fn from(value: (u32, u32)) -> Self {
        let sz: SizePx = value.into();
        sz.into()
    }
}
