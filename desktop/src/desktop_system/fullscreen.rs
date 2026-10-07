//! Full Screen Mode (ADR 0014): an instance presentation state that scales the
//! instance's content toward the window instead of its regular panel scale.
//!
//! The mode exists per launcher — one value shared by all of its base instances,
//! persisted with the desktop configuration — and per assistant instance, where
//! it is temporary. It is a pure content scale, analogous to the nested-project
//! slot scale: launcher presentation and visor layout are untouched, and the
//! camera resolves a focused full-screen instance as pixel-perfect at that scale
//! automatically. Focus, navigation, zoom, and resize never change the mode; the
//! only mutator is the `ToggleFullScreenMode` command.

use massive_applications::InstanceId;
use massive_geometry::SizePx;

use super::DesktopSystem;

impl DesktopSystem {
    /// Flips `instance`'s temporary Full Screen Mode. Only assistant instances
    /// carry one; a base instance always follows its launcher.
    pub(super) fn toggle_assistant_full_screen_mode(&mut self, instance: InstanceId) {
        let Some(presenter) = self.aggregates.instances.get_mut(&instance) else {
            panic!("a toggled assistant instance has a presenter");
        };
        presenter.toggle_full_screen_mode();
    }
}

pub fn fullscreen_scale(panel_size: SizePx, view_size: SizePx) -> f64 {
    if !view_size.is_empty() {
        (panel_size.width as f64 / view_size.width as f64)
            .min(panel_size.height as f64 / view_size.height as f64)
    } else {
        1.0
    }
}
