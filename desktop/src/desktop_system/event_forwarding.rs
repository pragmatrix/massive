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

                // Hit test already returns view-local coordinates.

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
}
