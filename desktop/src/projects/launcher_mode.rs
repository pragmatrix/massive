//! Mode-dependent policy: decisions that hang off `LauncherMode`, named so each
//! call site reads as the rule it applies.

use massive_applications::InstanceId;
use massive_geometry::{SizePx, Transform};
use massive_layout::{LayoutAxis, Offset, Placement, Size};

use crate::desktop_system::place_container_children;
use crate::projects::{CHILD_SPACING, LauncherMode, LauncherPresenter};

/// Whether the launcher's instances draw their own content onto the launcher's
/// background: only visors project instances onto the panel, bands render each
/// instance as its own rectangle.
pub fn renders_instance_background(mode: LauncherMode) -> bool {
    matches!(mode, LauncherMode::Visor)
}

/// The fixed panel extent a launcher measures with: visors measure with their
/// default panel extent, bands measure from their children like any container.
pub fn panel_measurement(mode: LauncherMode, default_panel_size: SizePx) -> Option<Size<2>> {
    match mode {
        LauncherMode::Band => None,
        LauncherMode::Visor => Some(default_panel_size.into()),
    }
}

/// Whether child panels of the launcher may be hit outside the launcher's own
/// rect: only visors overflow their panel bounds.
pub fn includes_overflow_children_in_hit_testing(mode: LauncherMode) -> bool {
    matches!(mode, LauncherMode::Visor)
}

/// Whether keyboard focus inside `mode`'s panel re-runs the layout: visors
/// collapse around the focus anchor only when more than the anchor instance
/// exists.
pub fn relayouts_on_keyboard_focus(mode: LauncherMode, instance_count: usize) -> bool {
    matches!(mode, LauncherMode::Visor) && instance_count > 1
}

/// Places the launcher's child panels: bands pack horizontally, visors pack
/// around the presenter's focus anchor with visor arc geometry, so the pack
/// decision names the mode policy while the anchor-dependent placement stays on
/// the presenter.
pub fn place_panel_children(
    mode: LauncherMode,
    launcher: &LauncherPresenter,
    local_offset: Offset<2>,
    child_sizes: &[Size<2>],
    child_instances: &[InstanceId],
    expanded: bool,
    default_panel_size: SizePx,
) -> Vec<Placement<Transform, 2>> {
    match mode {
        LauncherMode::Band => place_container_children(
            LayoutAxis::HORIZONTAL,
            CHILD_SPACING,
            local_offset,
            child_sizes,
        ),
        LauncherMode::Visor => launcher.place_visor_panel_children(
            local_offset,
            child_sizes,
            child_instances,
            expanded,
            default_panel_size,
        ),
    }
}
