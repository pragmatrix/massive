use anyhow::Result;

use massive_applications::ViewEvent;
use massive_input::Event;
use massive_renderer::RenderGeometry;

use super::change::{Changes, DesktopChange, set_focus};
use super::effects::DesktopEffect;
use super::keyboard_focus::WindowFocusRequest;
use super::{
    DesktopSystem, DesktopTarget, FocusableTarget, KeyInput, KeyboardFocusReason, ZoomLevel,
};
use crate::desktop_system::TargetLayout;
use crate::event_router::RouterStep;
use crate::hit_tester::HitTester;
use crate::instance_manager::InstanceManager;
use crate::targeted_event::EventTransition;

impl DesktopSystem {
    // This processes input events and converts it to a set of commands.
    pub fn process_input_event(
        &mut self,
        event: &Event<ViewEvent>,
        render_geometry: &RenderGeometry,
    ) -> Result<Changes> {
        let hit_tester = HitTester::new(
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
                RouterStep::PointerPressed(target) => {
                    changes += pointer_press_focus(target);
                }
                RouterStep::WindowFocusChanged(focused) => {
                    // Robustness: What about pointer focus?
                    if let Some(request) = self.keyboard_focus.set_window_focus(focused) {
                        let target = match request {
                            WindowFocusRequest::Clear => None,
                            WindowFocusRequest::Restore(target) => Some(target),
                        };
                        changes += set_focus(target, KeyboardFocusReason::InputTransition);
                    }
                }
                RouterStep::KeyboardInput(delivered) => {
                    changes += self.deliver_keyboard_input(delivered, event)?;
                }
                RouterStep::DeliverInput {
                    target,
                    event: delivered,
                } => {
                    changes <<= DesktopChange::ForwardEvents(
                        EventTransition::Send(target, delivered).into(),
                    );
                }
            }
        }

        Ok(changes)
    }

    /// Keys walk the keyboard focus path (ADR 0020); IME input and modifier changes go to the
    /// keyboard focus.
    fn deliver_keyboard_input(
        &self,
        delivered: ViewEvent,
        event: &Event<ViewEvent>,
    ) -> Result<Changes> {
        let Some(focused) = self.focused_target() else {
            return Ok(Changes::Empty);
        };
        if let Some(input) = KeyInput::from_view_event(&delivered, event.keyboard_modifiers()) {
            return self.deliver_key_along_focus_path(&input);
        }

        let mut transitions = vec![EventTransition::Send(focused.clone(), delivered.clone())];
        // Modifiers alter pointer interaction (Cmd activates hyperlinks), so a pointed-at target
        // other than the keyboard focus needs them too.
        if matches!(delivered, ViewEvent::ModifiersChanged(_))
            && let Some(pointer_focus) = self.event_router.pointer_focus()
            && *pointer_focus != focused
        {
            transitions.push(EventTransition::Send(pointer_focus.clone(), delivered));
        }
        Ok(DesktopChange::ForwardEvents(transitions.into()).into())
    }

    pub(super) fn focus(
        &mut self,
        target: Option<FocusableTarget>,
        instance_manager: &InstanceManager,
    ) -> Result<()> {
        let focus_change = self.keyboard_focus.focus(target);

        // Layout follows this change later, when `transact` syncs the layout focus (ADR 0002). The
        // camera move follows from the effects marked here (ADR 0043). Navigation affinity resets
        // are emitted as `SetNavigationAffinity(None)` sibling changes by `set_focus_change`, not
        // applied here.
        if let Some(change) = &focus_change {
            // A target's size may depend on whether it holds the focus.
            for target in [change.from, change.to].into_iter().flatten() {
                DesktopEffect::Measure(target.into()).mark();
            }
            DesktopEffect::ZoomLevelChanged.mark();
        }
        if let Some(change) = focus_change {
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

    /// Moves the layout focus to the live keyboard focus (ADR 0002).
    ///
    /// The focused instance's launcher takes it as its focus anchor. Marks `Measure` for the
    /// launchers whose layout depends on the focus and that held the previous or hold the new
    /// layout focus.
    pub(super) fn sync_layout_focus(&mut self) {
        let live = self.focused_path().instance();
        let previous = self.layout_focus;
        if previous == live {
            return;
        }
        self.layout_focus = live;

        let hierarchy = &self.aggregates.hierarchy;
        // The previous instance may have stopped while the layout focus lagged behind.
        let previous_launcher = previous
            .filter(|instance| hierarchy.exists(&DesktopTarget::from(*instance)))
            .map(|instance| hierarchy.launcher_of_instance(instance));
        let live_launcher = live.map(|instance| hierarchy.launcher_of_instance(instance));

        if let Some((instance, launcher)) = live.zip(live_launcher) {
            self.aggregates
                .launchers
                .get_mut(&launcher)
                .expect("Launcher missing")
                .focus_anchor_instance = Some(instance);
        }

        let context = self.aggregates.layout_context(
            self.default_panel_size,
            self.window_state.inner_size,
            live,
        );
        for launcher in [previous_launcher, live_launcher].into_iter().flatten() {
            if self.aggregates.launchers[&launcher].relayouts_on_focus((), &context) {
                DesktopEffect::Measure(DesktopTarget::Launcher(launcher)).mark();
            }
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

    /// Retarget keyboard focus, and the focus the window focus restores, to the parent if its path
    /// is inside the subtree rooted at `target`.
    ///
    /// A removed subtree can still contain either focus; leaving it in place would dangle the
    /// keyboard focus at a removed node, now or when the window regains focus.
    pub(super) fn refocus_to_parent_if_path_contains(
        &mut self,
        target: &DesktopTarget,
        instance_manager: &InstanceManager,
    ) -> Result<()> {
        if self.focused_path().contains(target) {
            let parent = self.focusable_parent(target);
            self.focus(parent, instance_manager)?;
        }
        if let Some(restore) = self.keyboard_focus.window_restore()
            && self.path_of(&DesktopTarget::from(restore)).contains(target)
        {
            let parent = self.focusable_parent(target);
            self.keyboard_focus.set_window_restore(parent);
        }
        Ok(())
    }

    fn focusable_parent(&self, target: &DesktopTarget) -> Option<FocusableTarget> {
        self.aggregates.hierarchy.parent(target).map(|parent| {
            parent
                .as_focusable()
                .unwrap_or_else(|| panic!("Keyboard focus on an unfocusable parent: {parent:?}"))
        })
    }
}

/// A press focuses what the pressed target selects, and a press on nothing clears the focus.
pub(super) fn pointer_press_focus(target: Option<DesktopTarget>) -> Changes {
    let target = target.map(|target| target.selected_target());
    let mut changes = Changes::Empty;
    // Clicking a target frames it fully in (ADR 0018).
    if target.is_some() {
        changes <<= DesktopChange::SetZoomLevel(ZoomLevel::Focus);
    }
    changes += set_focus(target, KeyboardFocusReason::InputTransition);
    changes
}
