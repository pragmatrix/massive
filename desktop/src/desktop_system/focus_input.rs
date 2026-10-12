use std::collections::HashSet;

use anyhow::Result;

use massive_applications::ViewEvent;
use massive_input::Event;
use massive_renderer::RenderGeometry;

use super::change::{Changes, DesktopChange, set_focus};
use super::keyboard_focus::{KeyboardFocusChange, WindowFocusRequest};
use super::{DesktopSystem, DesktopTarget, KeyInput, KeyboardFocusReason, ZoomLevel};
use crate::event_router::RouterStep;
use crate::hit_tester::AggregateHitTester;
use crate::instance_manager::InstanceManager;
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
                RouterStep::PointerPressed(target) => {
                    changes += pointer_press_focus(target);
                }
                RouterStep::WindowFocusChanged(focused) => {
                    if let Some(request) = self.keyboard_focus.set_window_focus(focused) {
                        let target = match request {
                            WindowFocusRequest::Clear => None,
                            WindowFocusRequest::Restore(target) => Some(target.into()),
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

    /// Keys walk the keyboard focus path (ADR 0020); IME input goes to the keyboard focus, and
    /// modifier changes also reach a different pointer focus, which they alter.
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
        if matches!(delivered, ViewEvent::ModifiersChanged(_))
            && let Some(pointer_focus) = self.event_router.pointer_focus()
            && *pointer_focus != focused
        {
            transitions.push(EventTransition::Send(pointer_focus.clone(), delivered));
        }
        Ok(DesktopChange::ForwardEvents(transitions.into()).into())
    }

    pub(super) fn focus<'a>(
        &mut self,
        target: impl Into<Option<&'a DesktopTarget>>,
        instance_manager: &InstanceManager,
    ) -> Result<()> {
        let target = target.into().map(|target| {
            target
                .as_focusable()
                .unwrap_or_else(|| panic!("Keyboard focus on an unfocusable target: {target:?}"))
        });
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
    /// dangle the keyboard focus at a removed node.
    pub(super) fn refocus_to_parent_if_path_contains(
        &mut self,
        target: &DesktopTarget,
        instance_manager: &InstanceManager,
    ) -> Result<()> {
        if self
            .aggregates
            .hierarchy
            .path_contains_target(self.focused_target().as_ref(), target)
        {
            let parent = self.aggregates.hierarchy.parent(target).cloned();
            self.focus(parent.as_ref(), instance_manager)?;
        }
        Ok(())
    }
}

/// A press focuses what the pressed target selects, and a press on nothing clears the focus.
pub(super) fn pointer_press_focus(target: Option<DesktopTarget>) -> Changes {
    let target = target.map(|target| DesktopTarget::from(target.selected_target()));
    let mut changes = Changes::Empty;
    // Clicking a target frames it fully in (ADR 0018).
    if target.is_some() {
        changes <<= DesktopChange::SetZoomLevel(ZoomLevel::Focus);
    }
    changes += set_focus(target, KeyboardFocusReason::InputTransition);
    changes
}
