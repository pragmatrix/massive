//! This is the public scene representation, a graph of reference counted objects that are accessed
//! through handle types.
//!
//! An object is a value that has an id assigned to it and is referable by any other objects.
//!
//! It maintains referential integrity using reference counting.
//!
//! Internal representations are not visible nor accessible here. They are put into a change tracker
//! and forwarded to the renderer. This is because the renderer needs a different representation of
//! the objects to be efficient.
//!
//! What's unique about this design is:
//! - The objects' lifetime are defined at this point and not in the renderer. By using reference
//!   counting, the referential integrity is guaranteed. This way, the renderer does not need to
//!   care about that.
//! - No values are stored in the client part of the application. They are directly forwarded to the
//!   renderer, this way excessive cloning can be avoided.
//! - All changes are pooled and transferred manually at once, so that intermediate states are not
//!   visible to renderer.
//! - Because lifetime is defined here, id generation is done inside the clients, too. Ids _are_
//!   opaque and are optimized for the renderer. The renderer prefers contiguous ids, so that it can
//!   use simple arrays to store data (imagine database tables). This has also the advantage that
//!   the renderer minimizes allocations and can trivially associate arbitrary additional data like
//!   buffers or caches that are needed to render the objects fast and with a low memory
//!   footprint and allocations.
use std::{any::TypeId, cell::RefCell, collections::HashMap, rc::Rc};

use anyhow::Result;
use derive_more::From;

use massive_geometry as geometry;
use massive_shapes::{GlyphRun, Quads};

mod change_tracker;
mod handle;
mod id;

pub use change_tracker::*;
pub use handle::*;
pub use id::Id;
use id::*;
use tokio::sync::mpsc;

/// A director is the only direct connection to the renderer. It tracks all the changes to scene
/// graph and uploads it on demand.
#[derive(Debug)]
pub struct Director {
    // Each type requires its own id generator to ensure that the generated ids are contiguous
    // within that type.
    id_generators: HashMap<TypeId, IdGen>,
    change_tracker: Rc<RefCell<ChangeTracker>>,
    upload_channel: mpsc::Sender<Vec<SceneChange>>,
}

impl Director {
    pub fn new(upload_channel: mpsc::Sender<Vec<SceneChange>>) -> Self {
        Self {
            id_generators: Default::default(),
            change_tracker: Default::default(),
            upload_channel,
        }
    }

    pub fn cast<T: Object + 'static>(&mut self, value: T) -> Handle<T> {
        let ti = TypeId::of::<T>();
        let id = self.id_generators.entry(ti).or_default().allocate();
        Handle::new(id, value, self.change_tracker.clone())
    }

    /// Send changes to the renderer.
    pub fn action(&mut self) -> Result<()> {
        let changes = self.change_tracker.borrow_mut().take_all();

        // Free up all deleted ids (this is done immediately for now, but may be later done in the
        // renderer, for example to keep ids alive until animations are finished or cached resources
        // are cleaned up)
        for (type_id, id) in changes.iter().flat_map(|sc| sc.destructive_change()) {
            // TODO: order by TypeId first?
            self.id_generators
                .get_mut(&type_id)
                .expect("Internal Error: Freeing an id failed, generator missing for type")
                .free(id)
        }

        Ok(self.upload_channel.try_send(changes)?)
    }
}

pub type Matrix4 = Handle<geometry::Matrix4>;

impl Object for geometry::Matrix4 {
    type Pinned = ();
    type Uploaded = Self;

    fn split(self) -> (Self::Pinned, Self::Uploaded) {
        ((), self)
    }

    fn promote_change(change: Change<Self::Uploaded>) -> SceneChange {
        SceneChange::Matrix(change)
    }
}

#[derive(Debug)]
pub struct PositionedShape {
    pub matrix: Matrix4,
    pub shape: Shape,
}

#[derive(Debug)]
pub struct PositionedRenderShape {
    pub matrix: Id,
    pub shape: Shape,
}

impl Object for PositionedShape {
    // We keep the matrix here.
    type Pinned = Matrix4;
    // And upload the render shape.
    type Uploaded = PositionedRenderShape;

    fn split(self) -> (Self::Pinned, Self::Uploaded) {
        let PositionedShape { matrix, shape } = self;
        let shape = PositionedRenderShape {
            matrix: matrix.id(),
            shape,
        };
        (matrix, shape)
    }

    fn promote_change(change: Change<Self::Uploaded>) -> SceneChange {
        SceneChange::PositionedShape(change)
    }
}

impl PositionedShape {
    pub fn new(matrix: Matrix4, shape: impl Into<Shape>) -> Self {
        Self {
            matrix,
            shape: shape.into(),
        }
    }
}

#[derive(Debug, From)]
pub enum Shape {
    GlyphRun(GlyphRun),
    Quads(Quads),
}
