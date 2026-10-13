//! Mode-dependent policy: decisions that hang off `LauncherMode`, named so each
//! call site reads as the rule it applies. The launcher's own layout decides its
//! mode-dependent rules in its `TargetLayout` impl (ADR 0022).

use crate::projects::LauncherMode;

/// Whether the launcher's instances draw their own content onto the launcher's
/// background: only visors project instances onto the panel, bands render each
/// instance as its own rectangle.
pub fn renders_instance_background(mode: LauncherMode) -> bool {
    matches!(mode, LauncherMode::Visor)
}

/// Whether child panels of the launcher may be hit outside the launcher's own
/// rect: only visors overflow their panel bounds.
pub fn includes_overflow_children_in_hit_testing(mode: LauncherMode) -> bool {
    matches!(mode, LauncherMode::Visor)
}
