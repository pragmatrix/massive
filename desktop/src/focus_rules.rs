use anyhow::Result;

use massive_applications::{InstanceChange, ViewRole};

use crate::{
    DesktopTarget, EventRouter, desktop_system::DesktopCommand, event_router::EventTransitions,
};

pub enum FocusEvent {
    FocusChanged(EventTransitions<DesktopTarget>),
}

#[derive(Debug)]
pub struct FocusRules<'a> {
    event_router: &'a mut EventRouter<DesktopTarget>,
}

impl FocusRules<'_> {
    pub fn pre(&mut self, _cmd: &DesktopCommand) -> Result<()> {
        Ok(())
    }

    pub fn post(&mut self, cmd: &DesktopCommand) -> Result<Option<FocusEvent>> {
        match cmd {
            DesktopCommand::Project(_) => {}
            DesktopCommand::StartInstance { .. } => {}
            DesktopCommand::StopInstance(_) => {}
            DesktopCommand::IntegrateInstanceSubmission(instance, instance_submission) => {
                for change in instance_submission.changes() {
                    match change {
                        InstanceChange::Scene(_) => {}
                        InstanceChange::CreateView(creation_info) => {
                            // If this instance is currently focused and the new view is primary, make it
                            // foreground so that the view is focused.

                            let focused = self.event_router.focused().cloned();

                            if let (
                                Some(DesktopTarget::Instance(focused_instance)),
                                ViewRole::Primary,
                            ) = (focused, &creation_info.role)
                                && focused_instance == *instance
                            {
                                let target = &DesktopTarget::View(creation_info.id);
                                return Ok(Some(FocusEvent::FocusChanged(
                                    self.event_router.focus(target),
                                )));
                            }
                        }
                        InstanceChange::View(_, _) => {}
                        InstanceChange::DestroyView(_) => {}
                        InstanceChange::End(_) => {}
                    }
                }
            }
            DesktopCommand::ZoomIn => {}
            DesktopCommand::ZoomOut => {}
            DesktopCommand::NavigateTo(_) => {}
            DesktopCommand::Navigate(_) => {}
        }

        Ok(None)
    }
}
