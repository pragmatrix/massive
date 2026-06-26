use anyhow::Result;
use winit::event::ElementState;
use winit::keyboard::{Key, NamedKey};

use massive_applications::ViewEvent;
use massive_input::Event;
use massive_renderer::RenderGeometry;

use super::navigation::Direction;
use super::{
    Cmd, DesktopCommand, DesktopSystem, DesktopTarget, Effects, FocusReason,
    POINTER_FEEDBACK_REENABLE_MAX_DURATION, POINTER_FEEDBACK_REENABLE_MIN_DISTANCE_PX,
};
use crate::event_router::{EventTransitions, NavigationTarget, ProcessOutcome};
use crate::hit_tester::AggregateHitTester;
use crate::instance_manager::InstanceManager;
use crate::targeted_event::TargetedEvent;

impl DesktopSystem {
    pub fn process_input_event(
        &mut self,
        event: &Event<ViewEvent>,
        instance_manager: &InstanceManager,
        render_geometry: &RenderGeometry,
    ) -> Result<Cmd> {
        let keyboard_cmd = self.process_keyboard_shortcuts(event)?;

        let cmd = if !keyboard_cmd.is_none() {
            keyboard_cmd
        } else {
            let hit_tester = AggregateHitTester::new(
                &self.aggregates.hierarchy,
                &self.layout_state,
                &self.aggregates.launchers,
                render_geometry,
            );

            match self.event_router.process(event, &hit_tester)? {
                ProcessOutcome::Transitions(transitions) => self
                    .apply_and_forward_focus_transitions(
                        transitions,
                        instance_manager,
                        FocusReason::InputTransition,
                    )?,
                ProcessOutcome::Focus(target) => DesktopCommand::NavigateTo(target).into(),
            }
        };

        self.update_pointer_feedback(event);

        Ok(cmd)
    }

    fn update_pointer_feedback(&mut self, event: &Event<ViewEvent>) {
        // Mode switch:
        // - keyboard press disables pointer-driven feedback
        // - mouse button/wheel re-enables immediately
        // - cursor movement re-enables only when movement is deliberate (distance + time gate)
        match (self.pointer_feedback_enabled, event.event()) {
            (
                true,
                ViewEvent::KeyboardInput {
                    event: key_event, ..
                },
            ) if key_event.state == ElementState::Pressed && !key_event.repeat => {
                self.pointer_feedback_enabled = false;
            }
            (false, ViewEvent::MouseInput { .. } | ViewEvent::MouseWheel { .. }) => {
                self.pointer_feedback_enabled = true;
            }
            (false, ViewEvent::CursorMoved { .. })
                if event.cursor_has_velocity(
                    POINTER_FEEDBACK_REENABLE_MIN_DISTANCE_PX,
                    POINTER_FEEDBACK_REENABLE_MAX_DURATION,
                ) =>
            {
                self.pointer_feedback_enabled = true;
            }
            _ => {}
        }
    }

    pub(super) fn focus(
        &mut self,
        target: &DesktopTarget,
        instance_manager: &InstanceManager,
        reason: FocusReason,
    ) -> Result<Effects> {
        let cmd = self.navigate_to(
            Some(NavigationTarget {
                target: target.clone(),
                event: None,
            }),
            instance_manager,
            reason,
        )?;

        // Invariant: Programmatic focus changes must not trigger commands.
        assert!(cmd.is_none());

        Ok(Effects::None)
    }

    pub(super) fn navigate_to(
        &mut self,
        target: Option<NavigationTarget<DesktopTarget>>,
        instance_manager: &InstanceManager,
        reason: FocusReason,
    ) -> Result<Cmd> {
        let transitions = self
            .event_router
            .focus(target.as_ref().map(|target| &target.target));
        let mut cmd =
            self.apply_and_forward_focus_transitions(transitions, instance_manager, reason)?;

        if let Some(NavigationTarget {
            target,
            event: Some(event),
        }) = target
        {
            cmd += self.forward_event(TargetedEvent(target, event), instance_manager)?;
        }

        Ok(cmd)
    }

    fn apply_and_forward_focus_transitions(
        &mut self,
        transitions: EventTransitions<DesktopTarget>,
        instance_manager: &InstanceManager,
        reason: FocusReason,
    ) -> Result<Cmd> {
        if !transitions.keyboard_focus_change().is_empty() && reason.resets_navigation_affinity() {
            self.navigation_control.reset_all();
        }

        let transition_effects = self.apply_keyboard_focus_change_effects(&transitions);
        self.deferred_launcher_layout_effects += transition_effects;

        self.forward_event_transitions(transitions, instance_manager)
    }

    fn apply_keyboard_focus_change_effects(
        &mut self,
        transitions: &EventTransitions<DesktopTarget>,
    ) -> Effects {
        // Architecture: A keyboard focus change should emit `UpdateCamera` directly here so
        // the camera recomputes from the new focus without depending on a layout `Place` pass
        // running. Today the camera only moves because `transaction_effects` unconditionally
        // measures the root, which drives a `Place` that emits `UpdateCamera`. Emitting it
        // here would let that root measure be removed. See `transaction_effects`.
        self.invalidate_layout_for_focus_change(transitions.keyboard_focus_change())
    }

    // Sync the focused instance's launcher visor anchor to the live focus. Idempotent and skipped
    // while the camera is locked; callers defer it until the camera unlocks.
    pub(super) fn sync_focused_launcher_anchor(&mut self) {
        let Some(instance_id) = self.focused_path().instance() else {
            return;
        };
        let Some(launcher_id) = self.instance_launcher(instance_id) else {
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
    ) -> Result<Effects> {
        if self
            .aggregates
            .hierarchy
            .path_contains_target(self.event_router.pointer_focus(), target)
        {
            let transitions = self.event_router.unfocus_pointer()?;
            assert!(
                self.forward_event_transitions(transitions, instance_manager)?
                    .is_none()
            );
        }
        Ok(Effects::None)
    }

    #[allow(unused)]
    fn refocus_pointer(
        &mut self,
        instance_manager: &InstanceManager,
        render_geometry: &RenderGeometry,
    ) -> Result<Cmd> {
        let transitions = self
            .event_router
            .reset_pointer_focus(&AggregateHitTester::new(
                &self.aggregates.hierarchy,
                &self.layout_state,
                &self.aggregates.launchers,
                render_geometry,
            ))?;

        self.forward_event_transitions(transitions, instance_manager)
    }

    fn process_keyboard_shortcuts(&self, event: &Event<ViewEvent>) -> Result<Cmd> {
        // Catch `CMD+t` and `CMD+w` if an instance has the keyboard focus.

        if let ViewEvent::KeyboardInput {
            event: key_event, ..
        } = event.event()
            && key_event.state == ElementState::Pressed
            && !key_event.repeat
            && event.device_states().is_command()
        {
            let focused = self.focused_path();

            // Simplify: Instance should probably return the launcher, too now.
            if let Some(instance) = focused.instance()
                && let Some(DesktopTarget::Launcher(launcher)) =
                    self.aggregates.hierarchy.parent(&instance.into())
            {
                let launcher_id = *launcher;
                match &key_event.logical_key {
                    Key::Character(c) if c.as_str() == "t" => {
                        return Ok(DesktopCommand::StartInstance {
                            launcher: launcher_id,
                            parameters: Default::default(),
                        }
                        .into());
                    }
                    Key::Character(c) if c.as_str() == "w" => {
                        // Architecture: Shouldn't this just end the current view, and let the
                        // instance decide then?
                        return Ok(DesktopCommand::StopInstance(instance).into());
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
                        return Ok(DesktopCommand::ZoomIn.into());
                    }

                    if direction == Direction::Down {
                        return Ok(DesktopCommand::ZoomOut.into());
                    }
                }
                return Ok(DesktopCommand::Navigate(direction).into());
            }
        }

        Ok(Cmd::None)
    }
}
