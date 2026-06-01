use anyhow::{Context, Result};
use derive_more::{From, Into};
use log::debug;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::mpsc::error::SendError;
use uuid::Uuid;
use winit::window::CursorIcon;

use massive_geometry::{BoxPx, SizePx, Vector3};
use massive_renderer::{RenderSubmission, RenderTarget};
use massive_scene::{Handle, Location, Object, Ref, ToLocation, Transform};

use crate::instance_context::InstanceCommand;
use crate::{InstanceId, Scene, ViewId};

/// ADR: Decided to let the View own the Scene, so that we do have a lifetime restriction on the
/// Scene and can properly clean up and detect dangling handles in this scene in the Desktop.
#[derive(Debug)]
pub struct View {
    command_sender: UnboundedSender<(InstanceId, InstanceCommand)>,
    instance: InstanceId,
    scene: Scene,
    id: ViewId,
    location: Handle<Location>,
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
        command_sender: UnboundedSender<(InstanceId, InstanceCommand)>,
        instance: InstanceId,
        parent: Ref<Location>,
        extents: BoxPx,
        scene: Scene,
        role: ViewRole,
    ) -> Result<Self> {
        let id = ViewId(Uuid::new_v4());

        let size: SizePx = extents.size().cast();
        let center_x = (size.width / 2) as f64;
        let center_y = (size.height / 2) as f64;
        let local_transform =
            Transform::from_translation(Vector3::new(-center_x, -center_y, 0.0)).enter(&scene);
        let location = local_transform
            .to_location()
            .relative_to(parent)
            .enter(&scene);

        command_sender.send((
            instance,
            InstanceCommand::CreateView(ViewCreationInfo { id, role, extents }),
        ))?;

        Ok(Self {
            command_sender,
            instance,
            scene,
            id,
            location,
        })
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The location's transform.
    ///
    /// This should not be modified
    // Architecture: Introduce a kind of Immutable handle or read only Handle.
    pub fn transform(&self) -> Ref<Transform> {
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

    pub fn render(&self) -> Result<()> {
        let submission = self.scene.begin_frame();
        self.command_sender
            .send((
                self.instance,
                InstanceCommand::View(self.id, ViewCommand::Render { submission }),
            ))
            .context("Failed to send a render request")
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
    Render { submission: RenderSubmission },
    /// Feature: This should probably specify a depth too.
    Resize(BoxPx),
    /// Set the title of the view. The desktop decides how to display it.
    SetTitle(String),
    /// Set the cursor icon for the view.
    SetCursor(CursorIcon),
}

impl RenderTarget for View {
    fn render(&mut self, submission: RenderSubmission) -> Result<()> {
        self.command_sender
            .send((
                self.instance,
                InstanceCommand::View(self.id, ViewCommand::Render { submission }),
            ))
            .context("Failed to send a render request")
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
