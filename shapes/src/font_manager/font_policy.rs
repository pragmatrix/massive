use crate::engine::ShapingEngineKind;

/// How a font manager is built: which shaping engine shapes, and whether system fonts are
/// selectable.
///
/// The engine is a startup decision every client names, with no library-level default (ADR 0005).
/// System fonts are a selection source the application did not name, so they are opt-in: without
/// them the manager is *bare* and selects only fonts the application loaded itself.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct FontPolicy {
    engine: ShapingEngineKind,
    system_fonts: bool,
}

impl FontPolicy {
    pub fn new(engine: ShapingEngineKind, system_fonts: bool) -> Self {
        Self {
            engine,
            system_fonts,
        }
    }

    /// The bare policy: only fonts the application loaded itself are selectable. This is the
    /// default clients state, because a selectable face nobody named can win selection (ADR 0005).
    pub fn bare(engine: ShapingEngineKind) -> Self {
        Self::new(engine, false)
    }

    /// The policy that additionally makes system fonts selectable, for fallback coverage.
    pub fn system(engine: ShapingEngineKind) -> Self {
        Self::new(engine, true)
    }

    pub fn engine(self) -> ShapingEngineKind {
        self.engine
    }

    pub fn system_fonts(self) -> bool {
        self.system_fonts
    }
}
