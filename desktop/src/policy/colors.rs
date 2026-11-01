//! The desktop's colors.
//!
//! Each color was picked by taste. Every submodule groups the colors of one part of the desktop.

/// The window behind everything the desktop presents.
pub mod desktop {
    use massive_geometry::Color;

    pub const BACKGROUND: Color = Color::BLACK;
}

/// The outline drawn around the hovered or focused target.
pub mod hover_outline {
    use massive_geometry::Color;

    pub const OUTLINE: Color = Color::rgb_u32(0xff0000);
}

/// An instance and its title bar (ADR 0019).
pub mod instance {
    use massive_geometry::Color;

    pub const BACKGROUND: Color = Color::rgb_u32(0x282828);
    pub const PRIMARY_TITLE_BAR_BACKGROUND: Color = Color::rgb_u32(0x1f4f8f);
    /// Sets assistant instances apart from primary ones.
    pub const ASSISTANT_TITLE_BAR_BACKGROUND: Color = Color::rgb_u32(0x6a3d8f);
}

/// The text of instance title bars and project headers.
pub mod title_bar {
    use massive_geometry::Color;

    pub const TEXT: Color = Color::WHITE;
}

/// The badge that briefly shows the zoom level after it changes.
pub mod zoom_level_indicator {
    use massive_geometry::Color;

    pub const BACKGROUND: Color = Color::rgb_u32(0x181818);
    /// The background's opacity while the badge is fully shown.
    pub const BACKGROUND_OPACITY: f32 = 0.85;
    pub const TEXT: Color = Color::rgb_u32(0xf5f5f5);
}

/// A project's header.
pub mod project {
    use massive_geometry::Color;

    pub const HEADER_BACKGROUND: Color = Color::rgb_u32(0x1f4d3d);
}

/// The counterparts of the colors above for everything an MCP root covers: its header, the headers
/// of the projects below it, its launchers, and its instances' title bars (ADR 0036). They are
/// amber tones, so a user sees at a glance where agents may act. Primary and assistant title bars
/// stay distinguishable.
pub mod mcp {
    use massive_geometry::Color;

    pub const PROJECT_HEADER_BACKGROUND: Color = Color::rgb_u32(0x6b4a1a);
    pub const LAUNCHER_BACKGROUND: Color = Color::rgb_u32(0x4a2a10);
    pub const PRIMARY_TITLE_BAR_BACKGROUND: Color = Color::rgb_u32(0x8f5a1f);
    pub const ASSISTANT_TITLE_BAR_BACKGROUND: Color = Color::rgb_u32(0x8f3d4f);
}

/// A launcher's background and name.
pub mod launcher {
    use massive_geometry::Color;

    // TODO: Need proper color palettes for UI elements.
    // spellcheck: ignore
    // const ALICE_BLUE: Color = Color::rgb_u32(0xf0f8ff);
    // const POWDER_BLUE: Color = Color::rgb_u32(0xb0e0e6);
    const MIDNIGHT_BLUE: Color = Color::rgb_u32(0x191970);

    pub const BACKGROUND: Color = MIDNIGHT_BLUE;
    pub const TEXT: Color = Color::WHITE;
}
