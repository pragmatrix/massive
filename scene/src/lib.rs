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

mod change;
mod change_collector;
mod change_surface;
mod ergonomics;
mod handle;
mod id;
mod objects;
mod scene;
mod transform_resolver;
mod type_id_generator;

pub use change::*;
pub use change_collector::*;
pub use change_surface::*;
pub use ergonomics::*;
pub use handle::*;
pub use id::Id;
pub use objects::*;
pub use scene::Scene;
pub use transform_resolver::*;
pub use type_id_generator::id_generator;

// Re-exports

pub use massive_geometry::Transform;
