use std::sync::Arc;

use anyhow::Result;
use derive_more::{From, Into};
use uuid::Uuid;
use winit::window::CursorIcon;

use massive_geometry::{BoxPx, Size, SizePx};
use massive_scene::{Handle, Location, Object, Ref, ToLocation, Transform};

use crate::{InstanceChange, InstanceChangeCollector, Scene, ViewId};

/// ADR: Decided to let the View own the Scene, so that we do have a lifetime restriction on the
/// Scene and can properly clean up and detect dangling handles in this scene in the Desktop.
#[derive(Debug)]
pub struct View {
    scene: Scene,
    id: ViewId,
    transform: Handle<Transform>,
    location: Handle<Location>,
    change_collector: Arc<InstanceChangeCollector>,
    title: String,
    cursor: CursorIcon,
}

impl Drop for View {
    fn drop(&mut self) {
        self.change_collector
            .collect(InstanceChange::DestroyView(self.id));
    }
}

impl View {
    pub(crate) fn new(
        parent: Ref<Location>,
        extents: BoxPx,
        scene: Scene,
        role: ViewRole,
        change_collector: Arc<InstanceChangeCollector>,
    ) -> Result<Self> {
        let id = ViewId(Uuid::new_v4());

        let size: Size = SizePx::from(extents.size().cast()).into();
        let local_transform = Transform::from(-size.center()).enter(&scene);
        let location = local_transform.to_location().relative_to(parent).enter(&scene);

        change_collector.collect(InstanceChange::CreateView(ViewCreationInfo {
            id,
            role,
            extents,
        }));

        Ok(Self {
            scene,
            id,
            transform: local_transform,
            location,
            change_collector,
            title: String::new(),
            cursor: CursorIcon::default(),
        })
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The location's transform.
    pub fn transform(&self) -> Ref<Transform> {
        self.location().value().transform.clone()
    }

    /// A reference to the location that is used to position the view in the parent desktop space.
    pub fn location(&self) -> Ref<Location> {
        self.location.to_ref()
    }

    /// Updates the view's local origin for a desktop-assigned extent.
    pub fn set_extent(&self, size: SizePx) {
        let size: Size = size.into();
        self.transform
            .update_if_changed(Transform::from(-size.center()));
    }

    #[allow(unused)]
    fn resize(&mut self, new_extents: impl Into<ViewExtent>) {
        self.change_collector.collect(InstanceChange::View(
            self.id,
            ViewChange::Resize(new_extents.into().into()),
        ))
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        let title = title.into();
        if self.title == title {
            return;
        }

        self.title = title.clone();
        self.change_collector
            .collect(InstanceChange::View(self.id, ViewChange::SetTitle(title)))
    }

    pub fn set_cursor(&mut self, cursor: CursorIcon) {
        if self.cursor == cursor {
            return;
        }

        self.cursor = cursor;
        self.change_collector
            .collect(InstanceChange::View(self.id, ViewChange::SetCursor(cursor)))
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
pub enum ViewChange {
    /// Feature: This should probably specify a depth too.
    Resize(BoxPx),
    /// Set the title of the view. The desktop decides how to display it.
    SetTitle(String),
    /// Set the cursor icon for the view.
    SetCursor(CursorIcon),
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
