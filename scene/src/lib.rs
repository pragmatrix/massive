//! This is the public client-side representation of scene content: a graph of objects that are
//! accessed through handles and references.
//!
//! An object is a value that has an id assigned to it and is referable by any other objects. Its
//! [`Handle`] is its single owner and writer. A [`Ref`] keeps it alive without writing it.
//!
//! A handle sends each change of its object to the current task's change queue. The queues forward
//! the changes to the lifetime stage, which turns them into the changes the renderer applies. The
//! renderer keeps its own tables of the values it receives.
//!
//! The changes a task collects are its [`AnyCollector`] change queue: one per task, its change
//! type fixed at install time (ADR 0008). The word "scene" here names the content the renderer
//! materializes, not a client-side collector.
//!
//! What's unique about this design is:
//! - The objects' lifetime is decided before the renderer, by the [`LifetimeStage`]. It counts the
//!   handle and the references of each object, which guarantees referential integrity, so the
//!   renderer does not need to care about it (ADR 0040).
//! - A handle keeps its object's value, and the renderer receives a snapshot of it with each change.
//!   Shapes are shared through an `Arc`, so a snapshot does not copy them.
//! - All changes are pooled and transferred manually at once, so that intermediate states are not
//!   visible to renderer.
//! - Because objects are created here, id generation is done inside the clients, too. `Ids` _are_
//!   opaque and are optimized for the renderer. The renderer prefers contiguous ids, so that it can
//!   use simple arrays to store data (imagine database tables). This has also the advantage that
//!   the renderer minimizes allocations and can trivially associate arbitrary additional data like
//!   buffers or caches that are needed to render the objects fast and with a low memory
//!   footprint and allocations.

mod any_collector;
mod change;
mod change_surface;
mod current_queue;
mod ergonomics;
mod handle;
mod id;
pub mod id_generator;
mod lifetime_stage;
mod objects;
mod render_change;

pub use any_collector::AnyCollector;
pub use change::*;
pub use change_surface::*;
pub use current_queue::{CollectChange, register_current_queue};
pub use handle::*;
pub use id::Id;
pub use lifetime_stage::LifetimeStage;
pub use objects::*;
pub use render_change::*;
pub mod prelude;
pub use ergonomics::{Submit, UnsubmittedLocation};

use massive_util::{self as util};

// Re-exports
pub use massive_geometry::Transform;

pub type SceneChangeSet = util::ChangeSet<SceneChange>;
