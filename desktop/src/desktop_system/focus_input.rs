use std::collections::HashSet;

use anyhow::Result;
use uuid::Uuid;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, NamedKey};

use massive_applications::ViewEvent;
use massive_input::Event;
use massive_renderer::RenderGeometry;

use super::change::{Changes, DesktopChange, set_focus};
use super::{
    DesktopCommand, DesktopFocusPath, DesktopSystem, DesktopTarget, Direction, KeyboardFocusReason,
    ZoomDepth,
};
use crate::desktop_system::change::Zoom;
use crate::event_router::{FocusRequestSource, KeyboardFocusChange, RouterStep};
use crate::hit_tester::AggregateHitTester;
use crate::instance_manager::InstanceManager;
use crate::instance_presenter::InstanceKind;
use crate::projects::{LaunchProfileId, launcher_mode};
use crate::targeted_event::EventTransition;

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
            &self.aggregates.configuration,
            render_geometry,
        );

        let output = self.event_router.process(event, &hit_tester)?;
        let mut changes = Changes::Empty;
        for step in output.steps {
            match step {
                RouterStep::PointerFocusChanged(change) => {
                    changes <<= DesktopChange::ForwardEvents(EventTransition::from(change).into());
                }
                RouterStep::RequestKeyboardFocus { target, source } => {
                    // Clicking a target frames it fully in (ADR 0017).
                    if source == FocusRequestSource::PointerPress && target.is_some() {
                        changes <<= DesktopChange::CommitZoomDepth(ZoomDepth::Innermost);
                    }
                    changes += set_focus(target, KeyboardFocusReason::InputTransition);
                }
                RouterStep::DeliverInput { target, event } => {
                    changes <<=
                        DesktopChange::ForwardEvents(EventTransition::Send(target, event).into());
                }
            }
        }

        Ok(changes)
    }

    pub(super) fn focus<'a>(
        &mut self,
        target: impl Into<Option<&'a DesktopTarget>>,
        instance_manager: &InstanceManager,
    ) -> Result<()> {
        let focus_change = self.event_router.focus(target.into());

        // Focus-change relayout is deferred until the camera unlocks; queue the affected launcher
        // measures now and let `transact` drain them once buttons are released. The camera move
        // itself is driven by `transact` observing the focus change, not queued here.
        // Navigation affinity resets are emitted as `SetNavigationAffinity(None)` sibling changes
        // by `set_focus_change`, not applied here.
        if let Some(change) = focus_change {
            let measures = self.launcher_measures_for_focus_change(&change);
            self.deferred_focus_launcher_measures.extend(measures);

            // Invariant: Forwarding focus/unfocus transitions never produces commands.
            assert!(
                self.forward_event_transitions(
                    EventTransition::from(change).into(),
                    instance_manager
                )?
                .is_empty()
            );
        }

        Ok(())
    }

    /// Returns the launchers that must be re-laid-out when keyboard focus moves to/from the
    /// affected targets. The camera move itself follows from `transact` observing the focus change.
    fn launcher_measures_for_focus_change(
        &self,
        change: &KeyboardFocusChange<DesktopTarget>,
    ) -> HashSet<LaunchProfileId> {
        targets_affected_by_keyboard_focus_change(change)
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
        let launcher_id = topology.launcher_of_instance(focused_instance);
        let instance_count = topology
            .get_nested(&DesktopTarget::Launcher(launcher_id))
            .len();

        // Architecture: Passing instance_count here is weird.
        self.aggregates
            .configuration
            .launcher(launcher_id)
            .filter(|launcher| {
                launcher_mode::relayouts_on_keyboard_focus(launcher.mode, instance_count)
            })
            .map(|_| launcher_id)
    }

    // Sync the focused instance's launcher visor anchor to the live focus. Idempotent and skipped
    // while the camera is locked; callers defer it until the camera unlocks.
    pub(super) fn sync_focused_launcher_anchor(&mut self) {
        let Some(instance_id) = self.focused_path().instance() else {
            return;
        };
        let launcher_id = self.aggregates.hierarchy.launcher_of_instance(instance_id);
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
            && let Some(change) = self.event_router.unfocus_pointer()
        {
            assert!(
                self.forward_event_transitions(
                    EventTransition::from(change).into(),
                    instance_manager
                )?
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
            .path_contains_target(self.event_router.keyboard_focus(), target)
        {
            let parent = self.aggregates.hierarchy.parent(target).cloned();
            self.focus(parent.as_ref(), instance_manager)?;
        }
        Ok(())
    }

    /// Architecture: Somehow the desktop presenter should handle these (we need some kind of
    /// event delivery down / up?)
    ///
    /// Design: This function is mixing state checks with the key detection.
    pub fn match_desktop_keyboard_shortcut(
        &self,
        event: &Event<ViewEvent>,
    ) -> Option<DesktopCommand> {
        // Cmd+Enter focuses a launcher slot or instance, then starts only from a launcher.
        // Cmd+T starts from either, and Cmd+W closes an instance.

        let ViewEvent::KeyboardInput {
            event: key_event, ..
        } = event.event()
        else {
            return None;
        };
        if key_event.state != ElementState::Pressed || !event.device_states().is_command() {
            return None;
        }

        let focused_path = self.focused_path();
        if let Some(focused_target) = focused_path.last()
            && !key_event.repeat
            && let Some(shortcut) =
                self.match_focused_target_shortcut(key_event, event, &focused_path, focused_target)
        {
            return Some(shortcut);
        }

        if let Some(direction) = match &key_event.logical_key {
            Key::Named(NamedKey::ArrowLeft) => Some(Direction::Left),
            Key::Named(NamedKey::ArrowRight) => Some(Direction::Right),
            Key::Named(NamedKey::ArrowUp) => Some(Direction::Up),
            Key::Named(NamedKey::ArrowDown) => Some(Direction::Down),
            _ => None,
        } {
            if event.device_states().is_ctrl() {
                match direction {
                    Direction::Up => {
                        return Some(DesktopCommand::Zoom(Zoom::In));
                    }
                    Direction::Down => {
                        return Some(DesktopCommand::Zoom(Zoom::Out));
                    }
                    _ => {}
                }
            }
            return Some(DesktopCommand::Navigate(direction));
        }
        None
    }

    fn match_focused_target_shortcut(
        &self,
        key_event: &KeyEvent,
        event: &Event<ViewEvent>,
        focused_path: &DesktopFocusPath,
        focused_target: &DesktopTarget,
    ) -> Option<DesktopCommand> {
        // Cmd+Enter

        if key_event.logical_key == Key::Named(NamedKey::Enter) {
            if self.is_fully_zoomed_in(focused_target, self.zoom_depth)
                && focused_target.wants_cmd_enter_when_focused()
            {
                return None;
            }
            return Some(DesktopCommand::Zoom(Zoom::Reset));
        }

        // Cmd + t

        if supports_instance_start_key(&key_event.logical_key, focused_target) {
            // `Shift` makes the new instance an assistant: it spawns without the launcher's
            // configured parameters and carries its own temporary Full Screen Mode (ADR 0014).
            let shift = event.device_states().is_shift();

            if let Some(instance) = focused_path.instance() {
                let launcher_id = self.aggregates.hierarchy.launcher_of_instance(instance);
                // Assistants open plain; base instances inherit the focused instance's parameters.
                let parameters = match shift {
                    false => self
                        .aggregates
                        .instances
                        .get(&instance)
                        .expect("Focused instance has no presenter")
                        .parameters()
                        .clone(),
                    true => Default::default(),
                };
                return Some(DesktopCommand::StartInstance {
                    launcher: launcher_id,
                    instance: Uuid::new_v4().into(),
                    root: None,
                    parameters,
                    kind: if shift {
                        InstanceKind::Assistant
                    } else {
                        InstanceKind::Base
                    },
                });
            }

            // Cmd + w

            if !shift
                && let Some(instance) = focused_path.instance()
                && let Key::Character(c) = &key_event.logical_key
                && c.as_str() == "w"
            {
                // Architecture: Shouldn't this just end the current view, and let the
                // instance decide then?
                return Some(DesktopCommand::StopInstance(instance));
            }
        }

        None
    }
}

fn supports_instance_start_key(key: &Key, target: &DesktopTarget) -> bool {
    match key {
        Key::Character(c) if c.as_str().eq_ignore_ascii_case("t") => {
            matches!(target, DesktopTarget::Instance(_) | DesktopTarget::View(_))
        }
        _ => false,
    }
}

fn targets_affected_by_keyboard_focus_change<T>(change: &KeyboardFocusChange<T>) -> Vec<&T> {
    [change.from.as_ref(), change.to.as_ref()]
        .into_iter()
        .flatten()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use massive_applications::InstanceId;

    #[test]
    fn launcher_start_keys_are_delegated_to_the_presenter() {
        let launcher = DesktopTarget::Launcher(Uuid::new_v4().into());
        let instance = DesktopTarget::Instance(InstanceId::from(Uuid::new_v4()));
        let view = DesktopTarget::View(massive_applications::ViewId::new());

        assert!(!supports_instance_start_key(
            &Key::Named(NamedKey::Enter),
            &launcher
        ));
        assert!(!supports_instance_start_key(
            &Key::Character("t".into()),
            &launcher
        ));
        assert!(!supports_instance_start_key(
            &Key::Named(NamedKey::Enter),
            &instance
        ));
        assert!(!supports_instance_start_key(
            &Key::Named(NamedKey::Enter),
            &view
        ));
        assert!(supports_instance_start_key(
            &Key::Character("t".into()),
            &instance
        ));
    }
}
