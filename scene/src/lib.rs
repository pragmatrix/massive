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
use tokio::sync::mpsc;

use id::*;

mod change_tracker;
mod handle;
mod id;
mod objects;

pub use change_tracker::*;
pub use handle::*;
pub use id::Id;
pub use objects::*;

/// A director is the only direct connection to the renderer. It tracks all the changes to scene
/// graph and uploads it on demand.
pub struct Director {
    // Each type requires its own id generator to ensure that the generated ids are contiguous
    // within that type.
    id_generators: HashMap<TypeId, IdGen>,
    change_tracker: Rc<RefCell<ChangeTracker>>,
    notify_changes: Box<dyn FnMut(Vec<SceneChange>) -> Result<()> + 'static>,
}

impl Director {
    pub fn from_sender(sender: mpsc::Sender<Vec<SceneChange>>) -> Self {
        Self::new(move |changes| Ok(sender.try_send(changes)?))
    }

    pub fn new(f: impl FnMut(Vec<SceneChange>) -> Result<()> + 'static) -> Self {
        Self {
            id_generators: Default::default(),
            change_tracker: Default::default(),
            notify_changes: Box::new(f),
        }
    }

    /// Put an object on the stage.
    pub fn stage<T: Object + 'static>(&mut self, value: T) -> Handle<T>
    where
        SceneChange: From<Change<T::Change>>,
    {
        let ti = TypeId::of::<T>();
        let id = self.id_generators.entry(ti).or_default().allocate();
        Handle::new(id, value, self.change_tracker.clone())
    }

    /// Send changes to the renderer.
    pub fn action(&mut self) -> Result<()> {
        let changes = self.change_tracker.borrow_mut().take_all();
        // Short circuit.
        if changes.is_empty() {
            return Ok(());
        }

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

        (self.notify_changes)(changes)
    }
}
