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
    /// A policy over `engine`, with system fonts selectable only when `system_fonts` is set.
    pub fn new(engine: ShapingEngineKind, system_fonts: bool) -> Self {
        Self {
            engine,
            system_fonts,
        }
    }

    pub fn engine(self) -> ShapingEngineKind {
        self.engine
    }

    pub fn system_fonts(self) -> bool {
        self.system_fonts
    }
}
