use anyhow::Result;
use winit::event::ElementState;
use winit::keyboard::{Key, NamedKey};

use massive_applications::ViewEvent;
use massive_input::Event;
use massive_renderer::RenderGeometry;

use super::{
    Cmd, DesktopCommand, DesktopSystem, DesktopTarget, POINTER_FEEDBACK_REENABLE_MAX_DURATION,
    POINTER_FEEDBACK_REENABLE_MIN_DISTANCE_PX, navigation::Direction,
};
use crate::focus_path::PathResolver;
use crate::hit_tester::AggregateHitTester;
use crate::instance_manager::InstanceManager;

impl DesktopSystem {
    pub fn process_input_event(
        &mut self,
        event: &Event<ViewEvent>,
        instance_manager: &InstanceManager,
        render_geometry: &RenderGeometry,
    ) -> Result<Cmd> {
        let keyboard_cmd = self.preprocess_keyboard_input(event)?;

        let cmd = if !keyboard_cmd.is_none() {
            keyboard_cmd
        } else {
            let hit_tester = AggregateHitTester::new(
                &self.aggregates.hierarchy,
                &self.layouter,
                &self.aggregates.launchers,
                &self.aggregates.instances,
                render_geometry,
            );

            let transitions = self.event_router.process(event, &hit_tester)?;
            self.invalidate_layout_for_focus_change(transitions.keyboard_focus_change());
            self.forward_event_transitions(transitions, instance_manager)?
        };

        self.update_pointer_feedback(event);

        Ok(cmd)
    }

    fn update_pointer_feedback(&mut self, event: &Event<ViewEvent>) {
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
    ) -> Result<()> {
        let transitions = self.event_router.focus(target);
        self.invalidate_layout_for_focus_change(transitions.keyboard_focus_change());

        // Invariant: Programmatic focus changes must not trigger commands.
        assert!(
            self.forward_event_transitions(transitions, instance_manager)?
                .is_none()
        );

        Ok(())
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
                    .is_none()
            );
        }
        Ok(())
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
                &self.layouter,
                &self.aggregates.launchers,
                &self.aggregates.instances,
                render_geometry,
            ))?;

        self.forward_event_transitions(transitions, instance_manager)
    }

    fn preprocess_keyboard_input(&self, event: &Event<ViewEvent>) -> Result<Cmd> {
        // Catch CMD+t and CMD+w if an instance has the keyboard focus.

        if let ViewEvent::KeyboardInput {
            event: key_event, ..
        } = event.event()
            && key_event.state == ElementState::Pressed
            && !key_event.repeat
            && event.device_states().is_command()
        {
            let focused = self.event_router.focused();
            let focused = self.aggregates.hierarchy.resolve_path(focused);

            // Simplify: Instance should probably return the launcher, too now.
            if let Some(instance) = focused.instance()
                && let Some(DesktopTarget::Launcher(launcher)) =
                    self.aggregates.hierarchy.parent(&instance.into())
            {
                match &key_event.logical_key {
                    Key::Character(c) if c.as_str() == "t" => {
                        return Ok(DesktopCommand::StartInstance {
                            launcher: *launcher,
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
                return Ok(DesktopCommand::Navigate(direction).into());
            }

            if let Key::Named(NamedKey::Escape) = &key_event.logical_key {
                return Ok(DesktopCommand::ZoomOut.into());
            }
        }

        Ok(Cmd::None)
    }
}
