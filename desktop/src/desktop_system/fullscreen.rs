use massive_applications::InstanceId;

use crate::desktop_system::DesktopSystem;
use crate::desktop_system::change::{DesktopChange, FullScreenChange};
use crate::instance_presenter::ViewPresentation;

use super::FocusDepth;

impl DesktopSystem {
    pub fn preferred_view_presentation(
        instance: InstanceId,
        focus_depth: FocusDepth,
        focused_instance: Option<InstanceId>,
    ) -> ViewPresentation {
        // If the instance is focused, it depends on the focus depth.
        if focused_instance == Some(instance) {
            if focus_depth == FocusDepth::InstanceFullScreen {
                return ViewPresentation::FullScreen;
            } else {
                return ViewPresentation::Regular;
            }
        }

        // Otherwise, it's always regular.
        ViewPresentation::Regular
    }

    pub fn sync_focus_event(
        &self,
        instance: InstanceId,
        desired: ViewPresentation,
    ) -> Option<DesktopChange> {
        let current = self.aggregates.instances[&instance].view_presentation()?;

        if current != desired {
            return Some(
                match desired {
                    ViewPresentation::Regular => FullScreenChange::Exit(instance),
                    ViewPresentation::FullScreen => FullScreenChange::Enter(instance),
                }
                .into(),
            );
        }

        None
    }
}
