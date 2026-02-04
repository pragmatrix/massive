use anyhow::{Context, Result};
use derive_more::{From, Into};
use log::debug;
use tokio::sync::mpsc::{UnboundedSender, error::SendError};

use uuid::Uuid;
use winit::window::CursorIcon;

use massive_geometry::{BoxPx, SizePx};
use massive_renderer::{RenderSubmission, RenderTarget};
use massive_scene::{Handle, Location, Object, ToLocation, Transform};

use crate::{InstanceId, Scene, ViewId, instance_context::InstanceCommand};

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
        extents: BoxPx,
        scene: Scene,
        role: ViewRole,
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
        let desktop_transform = Transform::IDENTITY.enter(&scene);
        let desktop_location = desktop_transform.to_location().enter(&scene);

        // The local transform is the basic center transform.
        //
        // Architecture: Do we need a local location anymore, if it does not make sense for the view
        // to modify it now that a full extents can be provided?
        let local_transform = Transform::IDENTITY.enter(&scene);
        let location = local_transform
            .to_location()
            .relative_to(&desktop_location)
            .enter(&scene);

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
