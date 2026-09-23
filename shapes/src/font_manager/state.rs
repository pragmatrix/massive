use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::Mutex;

use crate::engine::{FontRegistry, ShapingEngine, ShapingEngineKind};

pub struct FontManagerState {
    pub authority: Mutex<FontAuthority>,
    pub published: Arc<PublishedRegistry>,
}

/// The font-identity machinery: the canonical engine instance, used as the face authority.
///
/// The manager mutex covers only registration work (load, resolve, publish); shaping happens
/// per context, in shapers created from engine-built scratch state.
pub struct FontAuthority {
    /// Boxed to keep the manager handle small while supporting multiple engine implementations.
    pub engine: Box<dyn ShapingEngine>,
}

pub struct PublishedRegistry {
    pub kind: ShapingEngineKind,
    pub current: ArcSwap<FontRegistry>,
}
