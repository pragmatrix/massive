use std::path::PathBuf;

use anyhow::{Context, Result};
use log::error;
use massive_geometry::Identity;
use tokio::sync::mpsc::UnboundedSender;

use massive_scene::{Handle, Location, Matrix, SceneChanges};
use uuid::Uuid;
use winit::event::{self};

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

#[derive(Debug)]
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
