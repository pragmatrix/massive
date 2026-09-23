//! This is the public client-side representation of scene content: a graph of reference counted
//! objects that are accessed through handle types.
//!
//! An object is a value that has an id assigned to it and is referable by any other objects.
//!
//! It maintains referential integrity using reference counting.
//!
//! Internal representations are not visible nor accessible here. They are put into a change tracker
//! and forwarded to the renderer. This is because the renderer needs a different representation of
//! the objects to be efficient.
//!
//! The changes a task collects are its [`AnyCollector`] change queue: one per task, its change
//! type fixed at install time (ADR 0008). The word "scene" here names the content the renderer
//! materializes, not a client-side collector.
//!
//! What's unique about this design is:
//! - The objects' lifetime are defined at this point and not in the renderer. By using reference
//!   counting, the referential integrity is guaranteed. This way, the renderer does not need to
//!   care about that.
//! - No values are stored in the client part of the application. They are directly forwarded to the
//!   renderer, this way excessive cloning can be avoided.
//! - All changes are pooled and transferred manually at once, so that intermediate states are not
//!   visible to renderer.
//! - Because lifetime is defined here, id generation is done inside the clients, too. `Ids` _are_
//!   opaque and are optimized for the renderer. The renderer prefers contiguous ids, so that it can
//!   use simple arrays to store data (imagine database tables). This has also the advantage that
//!   the renderer minimizes allocations and can trivially associate arbitrary additional data like
//!   buffers or caches that are needed to render the objects fast and with a low memory
//!   footprint and allocations.
use std::fmt;

mod any_collector;
mod change;
mod change_surface;
mod ergonomics;
mod handle;
mod id;
mod objects;
mod scene;
mod transform_resolver;
mod type_id_generator;

pub use any_collector::AnyCollector;
pub use change::*;
pub use change_surface::*;
pub use handle::*;
pub use id::Id;
pub use objects::*;
pub mod prelude;
pub use ergonomics::UnenteredLocation;
pub use scene::enter;
pub use transform_resolver::*;
pub use type_id_generator::id_generator;

use massive_util::{self as util};

// Re-exports
pub use massive_geometry::Transform;

pub type ChangeCollector = util::ChangeCollector<SceneChange>;
pub type SceneChangeSet = util::ChangeSet<SceneChange>;

/// The erased receiver the `Handle<T>` type needs to propagate its changes and drops.
///
/// The trait indirection is here so that other layers can interleave scene changes into their
/// specific collector. Implementations retype the change into the collected change type; draining
/// is deliberately not part of this trait, it belongs to the concrete collector (ADR 0008).
pub trait ChangeSink: fmt::Debug + Send + Sync {
    fn send(&self, change: SceneChange);

    /// Send a batch of changes as one ordered batch.
    ///
    /// Required rather than defaulted: a sink that fell back to looping [`ChangeSink::send`] would
    /// take the queue's lock once per change, which the batch exists to avoid. Implementations
    /// must keep the iterator's order.
    ///
    /// The iterator is a `dyn` reference to keep this trait object-safe, which `Arc<dyn
    /// ChangeSink>` in every handle requires.
    fn send_all(&self, changes: &mut dyn Iterator<Item = SceneChange>);
}

/// Every `ChangeCollector` whose change type embeds scene changes is a `ChangeSink`.
impl<C> ChangeSink for util::ChangeCollector<C>
where
    C: From<SceneChange> + fmt::Debug + Send,
{
    fn send(&self, change: SceneChange) {
        self.collect(C::from(change));
    }

    fn send_all(&self, changes: &mut dyn Iterator<Item = SceneChange>) {
        self.collect_all(changes.map(C::from));
    }
}

// The blanket impl covers C = SceneChange via the reflexive From<SceneChange>; no separate
// impl exists.
// HandleChangeReceiver is gone: all impls and call sites moved to ChangeSink (ADR 0008).
