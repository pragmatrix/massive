//! The desktop's tuning values: sizes, durations, thresholds, and easing curves.
//!
//! None of these values follows from a structural reason. Each was picked by taste and usability
//! experience, so each could reasonably be different. Every submodule groups the values of one part
//! of the desktop. Colors live in `policy::colors`.

/// The animation of structural changes: instance layouts and visibility, slot layouts, title bars,
/// and camera moves.
pub mod structural_animation {
    use std::time::Duration;

    use massive_animation::Interpolation;

    pub const DURATION: Duration = Duration::from_millis(500);
    pub const INTERPOLATION: Interpolation = Interpolation::CubicOut;
}

/// The outline drawn around the hovered or focused target.
pub mod hover_outline {
    use std::time::Duration;

    use massive_animation::Interpolation;

    /// How long the outline takes to fade or follow a new target.
    pub const ANIMATION_DURATION: Duration = Duration::from_millis(250);
    pub const INTERPOLATION: Interpolation = Interpolation::CubicOut;
    /// The outline's stroke width and its outset from the target, in pixels.
    pub const STROKE: (f64, f64) = (10.0, 10.0);
}

/// When the pointer takes back its feedback after keyboard use.
///
/// The mouse must move intentionally, which means at least the minimum distance within the maximum
/// duration.
pub mod pointer_feedback {
    use std::time::Duration;

    pub const REENABLE_MIN_DISTANCE_PX: f64 = 24.0;
    pub const REENABLE_MAX_DURATION: Duration = Duration::from_millis(200);
}

/// The desktop's lifecycle.
pub mod lifecycle {
    use std::time::Duration;

    /// How long the shutdown waits for instances to stop.
    pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
}

/// The bar drawn above an instance's view (ADR 0019).
///
/// The sizes apply at a scale factor of 1.0 and follow the monitor's scale factor.
pub mod instance_title_bar {
    pub const HEIGHT_AT_1X: f64 = 24.0;
    pub const FONT_SIZE_AT_1X: f64 = 16.0;
    /// The horizontal indent of the title text.
    pub const INDENT_AT_1X: f64 = 8.0;
}

/// The badge that briefly shows the zoom level after it changes.
pub mod zoom_level_indicator {
    use std::time::Duration;

    use massive_animation::Interpolation;

    /// How long the badge is shown, including its fades.
    pub const DURATION: Duration = Duration::from_millis(1375);
    /// The badge fades in during the first 125ms, as a fraction of `DURATION`.
    pub const FADE_IN_END: f32 = 125.0 / 1375.0;
    /// The badge fades out during the last 250ms, as a fraction of `DURATION`.
    pub const FADE_OUT_START: f32 = 1125.0 / 1375.0;
    pub const FADE_INTERPOLATION: Interpolation = Interpolation::CubicOut;
    /// The badge's distance from the window's top right corner.
    pub const MARGIN: f64 = 16.0;
    pub const FONT_SIZE: f32 = 36.0;
    /// The horizontal and vertical padding around the label.
    pub const PADDING: (u32, u32) = (16, 12);
    pub const CORNER_RADIUS: f32 = 8.0;
}

/// Where the camera points.
pub mod camera {
    /// The margin the camera keeps around an extended selection on each side, as a fraction of the
    /// slot extent along the same axis (ADR 0032).
    pub const SELECTION_MARGIN_FRACTION: f64 = 0.25;
}

/// A project's header and the space around its matrix.
pub mod project {
    pub const HEADER_FONT_SIZE: f32 = 16.0 * 8.0;
    pub const HEADER_MIN_HEIGHT: u32 = 24;
    /// The gap between the header and the matrix.
    pub const HEADER_SPACING: u32 = 10;
    /// The padding around the project's contents.
    pub const PADDING: u32 = 10;
}

/// The matrix of slots in a project.
pub mod matrix {
    /// Column and row spacing, indexed by axis.
    pub const INTERVAL_SPACING: [u32; 2] = [10, 10];
}

/// A launcher and the instances it presents.
pub mod launcher {
    use std::time::Duration;

    use massive_animation::Interpolation;

    /// How long the launcher's background and name take to fade when an instance appears or the
    /// last one disappears.
    pub const FADING_DURATION: Duration = Duration::from_millis(500);
    pub const FADING_INTERPOLATION: Interpolation = Interpolation::CubicOut;
    pub const NAME_FONT_SIZE: f32 = 32.0 * 8.0;
    /// Band-mode panels are packed with no per-child spacing.
    pub const CHILD_SPACING: i32 = 0;
    /// How far a collapsed visor pushes its hidden non-anchor instances back.
    pub const COLLAPSED_NON_ANCHOR_Z_OFFSET: f64 = 1.0;
}

/// The arc on which a visor places its instances.
pub mod visor {
    use std::f64::consts::PI;

    /// The widest angle the arc reaches.
    pub const MAX_ARC_RADIANS: f64 = PI / 2.0;
    /// The instance count that controls how fast the arc approaches its widest angle.
    pub const ARC_SOFTNESS_INSTANCE_COUNT: f64 = 6.0;
}
