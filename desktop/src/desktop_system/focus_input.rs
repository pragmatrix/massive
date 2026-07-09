use std::collections::HashSet;

use anyhow::Result;
use uuid::Uuid;
use winit::event::ElementState;
use winit::keyboard::{Key, NamedKey};

use massive_applications::{InstanceId, ViewEvent};
use massive_input::Event;
use massive_renderer::RenderGeometry;

use super::navigation::Direction;
use super::{Commands, DesktopCommand, DesktopSystem, DesktopTarget, FocusReason};
use crate::EventTransition;
use crate::desktop_system::change::{Changes, DesktopChange, set_focus};
use crate::event_router::{EventTransitions, ProcessOutcome};
use crate::hit_tester::AggregateHitTester;
use crate::instance_manager::InstanceManager;
use crate::projects::LaunchProfileId;

impl DesktopSystem {
    // This processes input events and converts it to a set of commands.
    pub fn process_input_event(
        &mut self,
        event: &Event<ViewEvent>,
        render_geometry: &RenderGeometry,
    ) -> Result<Changes> {
        let hit_tester = AggregateHitTester::new(
            &self.aggregates.hierarchy,
            &self.layout_state,
            &self.aggregates.launchers,
            render_geometry,
        );

        let changes = match self.event_router.process(event, &hit_tester)? {
            ProcessOutcome::Transitions(transitions) => {
                DesktopChange::ForwardEvents(transitions).into()
            }
            ProcessOutcome::Focus(target) => {
                // The event router does not apply focus changes, we do.
                // Architecture: This should probably be done for pointer focus, too? Just for symmetry?
                if let Some(target) = target {
                    let t = target.target;
                    let mut changes: Changes =
                        set_focus(Some(t.clone()), FocusReason::InputTransition);

                    if let Some(event) = target.event {
                        changes +=
                            DesktopChange::ForwardEvents(EventTransition::Send(t, event).into())
                    }
                    changes
                } else {
                    set_focus(None, FocusReason::InputTransition)
                }
            }
        };

        Ok(changes)
    }

    pub(super) fn focus<'a>(
        &mut self,
        target: impl Into<Option<&'a DesktopTarget>>,
        instance_manager: &InstanceManager,
        _reason: FocusReason,
    ) -> Result<()> {
        let transitions = self.event_router.focus(target.into());

        // Focus-change relayout is deferred until the camera unlocks; queue the affected launcher
        // measures now and let `transact` drain them once buttons are released. The camera move
        // itself is driven by `transact` observing the focus change, not queued here.
        // Navigation affinity resets are emitted as `SetNavigationAffinity(None)` sibling changes
        // by `set_focus_change`, not applied here.
        if !targets_affected_by_keyboard_focus_change(&transitions).is_empty() {
            let measures = self.launcher_measures_for_focus_change(&transitions);
            self.deferred_focus_launcher_measures.extend(measures);
        }

        // Invariant: Forwarding focus/unfocus transitions never produces commands.
        assert!(
            self.forward_event_transitions(transitions, instance_manager)?
                .is_empty()
        );

        Ok(())
    }

    /// Returns the launchers that must be re-laid-out when keyboard focus moves to/from the
    /// affected targets. The camera move itself follows from `transact` observing the focus change.
    fn launcher_measures_for_focus_change(
        &self,
        transitions: &EventTransitions<DesktopTarget>,
    ) -> HashSet<LaunchProfileId> {
        targets_affected_by_keyboard_focus_change(transitions)
            .iter()
            .filter_map(|target| self.focus_target_launcher_for_layout(target))
            .collect()
    }

    /// Returns the launcher that should be re-laid-out when focus moves to/from `target`, or
    /// `None` if the target's launcher does not require focus-driven relayout.
    fn focus_target_launcher_for_layout(&self, target: &DesktopTarget) -> Option<LaunchProfileId> {
        let focused_path = self.path_of(target);
        let focused_instance = focused_path.instance()?;
        let topology = &self.aggregates.hierarchy;
        let launcher_id = topology.launcher_of_instance(focused_instance)?;
        let instance_count = topology
            .get_nested(&DesktopTarget::Launcher(launcher_id))
            .len();

        // Architecture: Passing instance_count here is weird.
        self.aggregates
            .launchers
            .get(&launcher_id)
            .filter(|launcher| launcher.should_relayout_on_keyboard_focus_change(instance_count))
            .map(|_| launcher_id)
    }

    // Sync the focused instance's launcher visor anchor to the live focus. Idempotent and skipped
    // while the camera is locked; callers defer it until the camera unlocks.
    pub(super) fn sync_focused_launcher_anchor(&mut self) {
        let Some(instance_id) = self.focused_path().instance() else {
            return;
        };
        let Some(launcher_id) = self.aggregates.hierarchy.launcher_of_instance(instance_id) else {
            return;
        };
        let launcher = self
            .aggregates
            .launchers
            .get_mut(&launcher_id)
            .expect("Launcher missing");
        if launcher.focus_anchor_instance != Some(instance_id) {
            launcher.focus_anchor_instance = Some(instance_id);
        }
    }

    pub(super) fn unfocus_pointer_if_path_contains(
        &mut self,
        target: &DesktopTarget,
        instance_manager: &InstanceManager,
    ) -> Result<()> {
        if self
            .aggregates
            .hierarchy
            .path_contains_target(self.event_router.pointer_focus(), target)
        {
            let transitions = self.event_router.unfocus_pointer()?;
            assert!(
                self.forward_event_transitions(transitions, instance_manager)?
                    .is_empty()
            );
        }
        Ok(())
    }

    /// Retarget keyboard focus to the parent if the focused path is inside the subtree rooted at
    /// `target`.
    ///
    /// A removed subtree can still contain the current keyboard focus; leaving it in place would
    /// dangle the router's `keyboard_focus` at a removed node.
    pub(super) fn refocus_to_parent_if_path_contains(
        &mut self,
        target: &DesktopTarget,
        instance_manager: &InstanceManager,
    ) -> Result<()> {
        if self
            .aggregates
            .hierarchy
            .path_contains_target(self.event_router.focused(), target)
        {
            let parent = self.aggregates.hierarchy.parent(target).cloned();
            self.focus(
                parent.as_ref(),
                instance_manager,
                FocusReason::InputTransition,
            )?;
        }
        Ok(())
    }

    pub fn match_desktop_keyboard_shortcut(
        &self,
        event: &Event<ViewEvent>,
    ) -> Option<DesktopKeyboardShortcut> {
        // Catch `CMD+t` and `CMD+w` if an instance has the keyboard focus.

        if let ViewEvent::KeyboardInput {
            event: key_event, ..
        } = event.event()
            && key_event.state == ElementState::Pressed
            && !key_event.repeat
            && event.device_states().is_command()
        {
            let focused_path = self.focused_path();

            // Simplify: Instance should probably return the launcher, too now.
            if let Some(instance) = focused_path.instance()
                && let Some(DesktopTarget::Launcher(launcher)) =
                    self.aggregates.hierarchy.parent(&instance.into())
            {
                let launcher_id = *launcher;
                match &key_event.logical_key {
                    Key::Character(c) if c.as_str() == "t" => {
                        return Some(DesktopKeyboardShortcut::NewInstance(launcher_id));
                    }
                    Key::Character(c) if c.as_str() == "w" => {
                        // Architecture: Shouldn't this just end the current view, and let the
                        // instance decide then?
                        return Some(DesktopKeyboardShortcut::CloseInstance(instance));
                    }
                    _ => {}
                }
            }

            if let Some(direction) = match &key_event.logical_key {
                Key::Named(NamedKey::ArrowLeft) => Some(Direction::Left),
                Key::Named(NamedKey::ArrowRight) => Some(Direction::Right),
                Key::Named(NamedKey::ArrowUp) => Some(Direction::Up),
                Key::Named(NamedKey::ArrowDown) => Some(Direction::Down),
                _ => None,
            } {
                if event.device_states().is_ctrl() {
                    if direction == Direction::Up {
                        return Some(DesktopKeyboardShortcut::ZoomIn);
                    }

                    if direction == Direction::Down {
                        return Some(DesktopKeyboardShortcut::ZoomOut);
                    }
                }
                return Some(DesktopKeyboardShortcut::Navigate(direction));
            }
        }

        None
    }
}

fn targets_affected_by_keyboard_focus_change<T>(this: &EventTransitions<T>) -> Vec<&T> {
    let mut touched = Vec::new();

    for transition in this.iter() {
        if let EventTransition::ChangeKeyboardFocus { from, to } = transition {
            if let Some(from) = from.as_ref() {
                touched.push(from);
            }
            if let Some(to) = to.as_ref() {
                touched.push(to);
            }
        }
    }

    touched
}

#[derive(Debug)]
pub enum DesktopKeyboardShortcut {
    NewInstance(LaunchProfileId),
    CloseInstance(InstanceId),
    ZoomOut,
    ZoomIn,
    Navigate(Direction),
}

impl DesktopKeyboardShortcut {
    pub fn into_command(self) -> DesktopCommand {
        match self {
            Self::NewInstance(launch_profile_id) => DesktopCommand::StartInstance {
                launcher: launch_profile_id,
                instance: Uuid::new_v4().into(),
                root: None,
                parameters: Default::default(),
            },
            Self::CloseInstance(instance) => DesktopCommand::StopInstance(instance),
            Self::ZoomOut => DesktopCommand::ZoomOut,
            Self::ZoomIn => DesktopCommand::ZoomIn,
            Self::Navigate(direction) => DesktopCommand::Navigate(direction),
        }
    }
}
