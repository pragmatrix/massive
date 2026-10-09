use anyhow::Result;
use log::warn;
use massive_applications::ViewEvent;

use super::{Aggregates, Commands, DesktopSystem, DesktopTarget};
use crate::focus_path::PathResolver;
use crate::instance_manager::InstanceManager;
use crate::projects::SlotContent;
use crate::targeted_event::{EventTransitions, TargetedEvent, convert_to_targeted_events};

impl DesktopSystem {
    pub(super) fn forward_event_transitions(
        &mut self,
        transitions: EventTransitions<DesktopTarget>,
        instance_manager: &InstanceManager,
    ) -> Result<Commands> {
        let mut commands = Commands::Empty;

        // Architecture: Don't use the keyboard_modifiers here, let the event_router provide them
        // when creating EventTransitions for focus changes so that they are available when needed
        // (and are in sync).
        let keyboard_modifiers = self.event_router.keyboard_modifiers();

        let targeted_events =
            convert_to_targeted_events(transitions, keyboard_modifiers, &self.aggregates.hierarchy);

        // Robustness: While we need to forward all transitions we currently process only one intent.
        for event in targeted_events {
            commands += self.forward_event(event, instance_manager)?;
        }

        Ok(commands)
    }

    /// Forward event transitions to the appropriate handler based on the target type.
    ///
    /// Design: I think we need to split the events sent to the view and the events delivered
    /// internally. This way, we can perhaps remove the access to the `InstanceManager` here.
    pub fn forward_event(
        &mut self,
        TargetedEvent(target, event): TargetedEvent<DesktopTarget>,
        instance_manager: &InstanceManager,
    ) -> Result<Commands> {
        // Design: This is the only place where we get the proper unfocus traversal for free. But I
        // don't like that.
        if matches!(&event, ViewEvent::Focused(false)) {
            self.remember_unfocused_project_slot(&target);
        }

        // Route to the appropriate handler based on the last target in the path
        match target {
            DesktopTarget::Launcher(launcher_id) => {
                // The configuration (params) and the presenter are separate fields of the
                // same aggregate struct, so split borrows keep both readable in one call.
                let Aggregates {
                    launchers,
                    configuration,
                    ..
                } = &mut self.aggregates;
                let params = &configuration[launcher_id];
                let launcher = launchers.get_mut(&launcher_id).expect("Launcher not found");
                return launcher.process(event, &params.params);
            }
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
                    return Ok(Commands::Empty);
                };

                // Hit testing already returned view-local coordinates.
                if let Err(e) = instance_manager.send_view_event((instance, view_id), event.clone())
                {
                    // This might happen when an instance ends, but we haven't yet received the
                    // information.
                    warn!("Sending view event {event:?} failed with {e}");
                }
            }
            _ => {}
        }

        Ok(Commands::Empty)
    }

    fn remember_unfocused_project_slot(&mut self, target: &DesktopTarget) {
        let content = match target {
            DesktopTarget::Launcher(launcher) => SlotContent::Launcher(*launcher),
            DesktopTarget::Project(project) => SlotContent::Project(*project),
            _ => return,
        };
        let Some(DesktopTarget::ProjectMatrix(project)) = self.aggregates.hierarchy.parent(target)
        else {
            return;
        };
        let project = *project;
        let placement = self
            .aggregates
            .configuration
            .project(project)
            .and_then(|record| record.placement_of_content(content))
            .expect("unfocused slot is present in project configuration");
        self.aggregates
            .projects
            .get_mut(&project)
            .expect("project has a presenter")
            .last_focused_placement = Some(placement);
    }
}
