use std::collections::HashSet;

use anyhow::Result;

use massive_applications::ViewEvent;
use massive_input::Event;
use massive_renderer::RenderGeometry;

use super::change::{Changes, DesktopChange, set_focus};
use super::keyboard_focus::{KeyboardFocusChange, WindowFocusRequest};
use super::{
    DesktopSystem, DesktopTarget, FocusableTarget, KeyInput, KeyboardFocusReason, ZoomLevel,
};
use crate::desktop_system::TargetLayout;
use crate::event_router::RouterStep;
use crate::hit_tester::HitTester;
use crate::instance_manager::InstanceManager;
use crate::projects::LaunchProfileId;
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
        change: &KeyboardFocusChange,
    ) -> HashSet<LaunchProfileId> {
        [change.from, change.to]
            .into_iter()
            .flatten()
            .filter_map(|target| self.focus_target_launcher_for_layout(&target.into()))
            .collect()
    }

    /// Returns the launcher that should be re-laid-out when focus moves to/from `target`, or
    /// `None` if the target's launcher does not require focus-driven relayout.
    fn focus_target_launcher_for_layout(&self, target: &DesktopTarget) -> Option<LaunchProfileId> {
        let focused_path = self.path_of(target);
        let focused_instance = focused_path.instance()?;
        let launcher_id = self
            .aggregates
            .hierarchy
            .launcher_of_instance(focused_instance);
        let context = self.aggregates.layout_context(
            self.default_panel_size,
            self.window_state.inner_size,
            self.focused_path().instance(),
        );
        self.aggregates.launchers[&launcher_id]
            .relayouts_on_focus((), &context)
            .then_some(launcher_id)
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
