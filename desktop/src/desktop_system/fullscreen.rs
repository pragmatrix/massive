use anyhow::Result;

use massive_applications::{InstanceId, ViewEvent};
use massive_geometry::SizePx;

use crate::desktop_system::DesktopSystem;
use crate::instance_manager::InstanceManager;
use crate::instance_presenter::InstancePresentation;
use crate::window_state::WindowState;

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

    pub fn apply_instance_presentation(
        &mut self,
        instance: InstanceId,
        window_state: &WindowState,
        instance_manager: &InstanceManager,
    ) -> Result<()> {
        let presentation = self.resolve_instance_presentation(instance, window_state.inner_size);
        if let Some((view, size)) = self
            .aggregates
            .instances
            .get_mut(&instance)
            .expect("Instance missing")
            .apply_presentation(presentation)
        {
            instance_manager.send_view_event((instance, view), ViewEvent::Resized(size))?;
        }
        Ok(())
    }
}
