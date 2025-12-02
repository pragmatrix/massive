use anyhow::Result;

use massive_applications::{InstanceId, ViewEvent, ViewRole};

use crate::{FocusManager, FocusTransition, instance_manager::InstanceManager};

#[derive(Debug, Default)]
pub struct UI {
    focus_manager: FocusManager,
}

impl UI {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn make_foreground(
        &mut self,
        instance: InstanceId,
        window_focused: bool,
        instance_manager: &mut InstanceManager,
    ) -> Result<()> {
        // If the window is not focus, we just focus the instance, but not the view for now.

        let focused_view = {
            if window_focused {
                instance_manager.get_view_by_role(instance, ViewRole::Primary)?
            } else {
                None
            }
        };

        let transitions = self.focus_manager.focus(instance, focused_view);
        Self::transition(transitions, instance_manager)
    }

    fn transition(
        transitions: Vec<FocusTransition>,
        instance_manager: &mut InstanceManager,
    ) -> Result<()> {
        for transition in transitions {
            match transition {
                FocusTransition::UnfocusView(instance, view) => {
                    instance_manager.send_view_event(instance, view, ViewEvent::Focused(false))?;
                }
                FocusTransition::FocusView(instance, view) => {
                    instance_manager.send_view_event(instance, view, ViewEvent::Focused(true))?;
                }
                FocusTransition::UnfocusInstance(_instance_id) => {}
                FocusTransition::FocusInstance(_instance_id) => {}
            }
        }
        Ok(())
    }
}
