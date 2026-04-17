use anyhow::Result;
use log::warn;

use crate::event_router::EventTransitions;
use crate::focus_path::PathResolver;
use crate::instance_manager::InstanceManager;
use crate::send_transition::{SendTransition, convert_to_send_transitions};

use super::{Cmd, DesktopSystem, DesktopTarget};

impl DesktopSystem {
    pub(super) fn forward_event_transitions(
        &mut self,
        transitions: EventTransitions<DesktopTarget>,
        instance_manager: &InstanceManager,
    ) -> Result<Cmd> {
        if self.pointer_feedback_enabled
            && let Some(pointer_focus) = transitions.pointer_focus_target()
        {
            self.sync_hover_rect_to_pointer_path(pointer_focus);
        }

        let mut cmd = Cmd::None;

        let keyboard_modifiers = self.event_router.keyboard_modifiers();

        let send_transitions = convert_to_send_transitions(
            transitions,
            keyboard_modifiers,
            &self.aggregates.hierarchy,
        );

        // Robustness: While we need to forward all transitions we currently process only one intent.
        for transition in send_transitions {
            cmd += self.forward_event_transition(transition, instance_manager)?;
        }

        Ok(cmd)
    }

    /// Forward event transitions to the appropriate handler based on the target type.
    fn forward_event_transition(
        &mut self,
        SendTransition(target, event): SendTransition<DesktopTarget>,
        instance_manager: &InstanceManager,
    ) -> Result<Cmd> {
        // Route to the appropriate handler based on the last target in the path
        match target {
            DesktopTarget::Desktop => {}
            DesktopTarget::Instance(..) => {}
            DesktopTarget::View(view_id) => {
                let path = self
                    .aggregates
                    .hierarchy
                    .resolve_path(Some(&view_id.into()));
                let Some(instance) = path.instance() else {
                    // This happens when the instance is gone (resolve_path returns only the view, because it puts it by default in the first position).
                    warn!(
                        "Instance of view {view_id:?} not found (path: {path:?}), can't deliver event: {event:?}"
                    );
                    return Ok(Cmd::None);
                };

                // Need to translate the event. The view has its own coordinate system.
                let event = if let Some(rect) = self.rect(&target) {
                    event.translate(-rect.origin())
                } else {
                    // This happens on startup on PresentView, because the layout isn't there yet.
                    event
                };

                if let Err(e) = instance_manager.send_view_event((instance, view_id), event.clone())
                {
                    // This might happen when an instance ends, but we haven't yet received the
                    // information.
                    warn!("Sending view event {event:?} failed with {e}");
                }
            }
            DesktopTarget::Group(..) => {}
            DesktopTarget::Launcher(launcher_id) => {
                let launcher = self
                    .aggregates
                    .launchers
                    .get_mut(&launcher_id)
                    .expect("Launcher not found");
                return launcher.process(event);
            }
        }

        Ok(Cmd::None)
    }

    pub(super) fn sync_hover_rect_to_pointer_path(
        &mut self,
        pointer_focus: Option<&DesktopTarget>,
    ) {
        let hover_rect = match pointer_focus {
            Some(DesktopTarget::Instance(instance_id)) => {
                self.rect(&DesktopTarget::Instance(*instance_id))
            }
            Some(DesktopTarget::View(view_id)) => match self
                .aggregates
                .hierarchy
                .parent(&DesktopTarget::View(*view_id))
            {
                Some(DesktopTarget::Instance(instance_id)) => {
                    self.rect(&DesktopTarget::Instance(*instance_id))
                }
                Some(_) => panic!("Internal error: View parent is not an instance"),
                None => None,
            },
            Some(DesktopTarget::Launcher(launcher_id)) => {
                self.rect(&DesktopTarget::Launcher(*launcher_id))
            }
            _ => None,
        };

        self.aggregates.project_presenter.set_hover_rect(hover_rect);
    }
}
