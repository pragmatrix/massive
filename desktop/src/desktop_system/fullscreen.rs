use massive_applications::InstanceId;
use massive_geometry::SizePx;

use crate::desktop_system::DesktopSystem;
use crate::instance_presenter::InstancePresentation;

use super::FocusDepth;

impl DesktopSystem {
    pub fn resolve_instance_presentation(
        &self,
        instance: InstanceId,
        window_size: SizePx,
    ) -> InstancePresentation {
        if self.focused_path().instance() == Some(instance)
            && self.focus_depth == FocusDepth::InstanceFullScreen
        {
            InstancePresentation::full_screen(self.default_panel_size, window_size)
        } else {
            InstancePresentation::regular(self.default_panel_size)
        }
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
