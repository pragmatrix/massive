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
        let regular_size = self.aggregates.instances[&instance].regular_size();
        if self.focused_path().instance() == Some(instance)
            && self.focus_depth == FocusDepth::InstanceFullScreen
        {
            InstancePresentation::full_screen(regular_size, window_size)
        } else {
            InstancePresentation::regular(regular_size)
        }
    }
}
