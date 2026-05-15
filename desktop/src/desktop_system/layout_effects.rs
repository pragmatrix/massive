use anyhow::Result;

use massive_animation::Interpolation;
use massive_applications::InstanceId;
use massive_geometry::{SizePx, Transform};
use massive_layout::Placement;

use super::effects::{DesktopEffect, DesktopEffectQueue, EffectContext, Effects};
use super::{DesktopLayoutAlgorithm, DesktopSystem, DesktopTarget};
use crate::focus_path::PathResolver;
use crate::instance_presenter::STRUCTURAL_ANIMATION_DURATION;
use crate::projects::LaunchProfileId;

impl DesktopSystem {
    pub(super) fn transaction_effects(&self, command_effects: Effects) -> Effects {
        let mut effects = Effects::from(DesktopEffect::UpdateLauncherExpansion);
        effects += command_effects;
        effects += DesktopEffect::RecomputeLayout(DesktopTarget::Desktop);
        effects
    }

    pub(super) fn run_effects_to_completion(
        &mut self,
        context: EffectContext,
        initial_effects: Effects,
    ) -> Result<()> {
        if context.lock_camera {
            // Lock camera motion immediately, including already running camera animations.
            let camera = self.camera.value();
            self.camera.set_immediately(camera);
        }

        let mut effects = DesktopEffectQueue::default();
        effects.enqueue_all(initial_effects);

        while let Some(effect) = effects.pop_front() {
            let follow_up = self.handle_effect(effect, context)?;
            effects.enqueue_all(follow_up);
        }

        Ok(())
    }

    fn handle_effect(&mut self, effect: DesktopEffect, context: EffectContext) -> Result<Effects> {
        match effect {
            DesktopEffect::UpdateLauncherExpansion => {
                self.update_launcher_expansion_effect(context);
                Ok(Effects::None)
            }
            DesktopEffect::RecomputeLayout(target) => Ok(DesktopEffect::MeasureNode(target).into()),
            DesktopEffect::MeasureNode(target) => self.handle_measure_node_effect(target),
            DesktopEffect::PlaceNode(root) => self.place_layout_effect(root),
            DesktopEffect::ApplyLayoutChanges => self.apply_layout_effect(context),
            DesktopEffect::UpdateCamera => {
                self.update_camera_effect(context);
                Ok(Effects::None)
            }
            DesktopEffect::SyncHover => {
                self.sync_hover_effect();
                Ok(Effects::None)
            }
        }
    }

    fn update_launcher_expansion_effect(&mut self, context: EffectContext) {
        let focused_target = self.event_router.focused().cloned();
        self.update_launcher_visor_expansion(focused_target.as_ref(), context.animate);
    }

    fn handle_measure_node_effect(&mut self, target: DesktopTarget) -> Result<Effects> {
        let focused_target = self.event_router.focused().cloned();
        let focused_instance = self
            .aggregates
            .hierarchy
            .resolve_path(focused_target.as_ref())
            .instance();
        let algorithm = DesktopLayoutAlgorithm {
            aggregates: &self.aggregates,
            default_panel_size: self.default_panel_size,
            focused_instance,
        };

        let missing_children = self
            .layout_state
            .missing_child_measures(&target, &self.aggregates.hierarchy);
        if !missing_children.is_empty() {
            let mut effects = Effects::None;
            for child in missing_children {
                effects += DesktopEffect::MeasureNode(child);
            }
            effects += DesktopEffect::MeasureNode(target);
            return Ok(effects);
        }

        let outcome = self
            .layout_state
            .measure_node(&target, &self.aggregates.hierarchy, &algorithm);

        let mut effects = Effects::None;
        if outcome.size_changed {
            if let Some(parent) = outcome.parent {
                effects += DesktopEffect::MeasureNode(parent);
            } else {
                effects += DesktopEffect::PlaceNode(target);
            }
        } else {
            effects += DesktopEffect::PlaceNode(target);
        }

        Ok(effects)
    }

    fn place_layout_effect(&mut self, root: DesktopTarget) -> Result<Effects> {
        let focused_target = self.event_router.focused().cloned();
        let focused_instance = self
            .aggregates
            .hierarchy
            .resolve_path(focused_target.as_ref())
            .instance();
        let algorithm = DesktopLayoutAlgorithm {
            aggregates: &self.aggregates,
            default_panel_size: self.default_panel_size,
            focused_instance,
        };

        self.layout_state
            .place_from_target(&root, &self.aggregates.hierarchy, &algorithm);

        Ok(DesktopEffect::ApplyLayoutChanges.into())
    }

    fn apply_layout_effect(&mut self, context: EffectContext) -> Result<Effects> {
        let changed = self.layout_state.take_staged_changed();
        self.apply_layout_changes(changed, context.animate);

        let mut effects = Effects::None;
        if context.permit_camera_moves {
            effects += DesktopEffect::UpdateCamera;
        }
        effects += DesktopEffect::SyncHover;
        Ok(effects)
    }

    fn update_camera_effect(&mut self, context: EffectContext) {
        let focused_target = self.event_router.focused().cloned();
        if let Some(focused) = focused_target.as_ref() {
            let camera = self.camera_for_focus(focused);
            if let Some(camera) = camera {
                if context.animate {
                    self.camera.animate_if_changed(
                        camera,
                        STRUCTURAL_ANIMATION_DURATION,
                        Interpolation::CubicOut,
                    );
                } else {
                    self.camera.set_immediately(camera);
                }
            }
        }
    }

    fn sync_hover_effect(&mut self) {
        let pointer_focus = if self.pointer_feedback_enabled {
            self.event_router.pointer_focus().cloned()
        } else {
            None
        };
        self.sync_hover_rect_to_pointer_path(pointer_focus.as_ref());
    }

    fn apply_layout_changes(
        &mut self,
        changed: Vec<(DesktopTarget, Placement<Transform, 2>)>,
        animate: bool,
    ) {
        for (id, placement) in changed {
            let layout_size = placement.rect.size;
            let size_px = SizePx::new(layout_size[0], layout_size[1]);
            let transform = placement.transform;

            match id {
                DesktopTarget::Desktop => {}
                DesktopTarget::Instance(instance_id) => {
                    self.aggregates
                        .instances
                        .get_mut(&instance_id)
                        .expect("Instance missing")
                        .set_layout(size_px, transform, animate);
                }
                DesktopTarget::Group(group_id) => {
                    self.aggregates
                        .groups
                        .get_mut(&group_id)
                        .expect("Missing group")
                        .size = size_px;
                }
                DesktopTarget::Launcher(launcher_id) => {
                    self.aggregates
                        .launchers
                        .get_mut(&launcher_id)
                        .expect("Launcher missing")
                        .set_layout(size_px, transform, animate);
                }
                DesktopTarget::View(..) => {
                    // Robustness: Support resize here?
                }
            }
        }
    }

    fn instance_launcher(&self, instance_id: InstanceId) -> Option<LaunchProfileId> {
        let instance_target = DesktopTarget::Instance(instance_id);
        match self.aggregates.hierarchy.parent(&instance_target) {
            Some(DesktopTarget::Launcher(id)) => Some(*id),
            _ => None,
        }
    }

    /// Marks launchers that need relayout due to a keyboard focus change as reflow-pending.
    pub(super) fn invalidate_layout_for_focus_change<'a>(
        &mut self,
        targets: impl IntoIterator<Item = &'a DesktopTarget>,
    ) -> Effects {
        let mut effects = Effects::None;
        for target in targets {
            if let Some(launcher_id) = self.focus_target_launcher_for_layout(target) {
                effects += DesktopEffect::RecomputeLayout(DesktopTarget::Launcher(launcher_id));
            }
        }
        effects
    }

    pub(super) fn defer_layout_for_focus_change<'a>(
        &mut self,
        targets: impl IntoIterator<Item = &'a DesktopTarget>,
    ) {
        let launcher_ids: Vec<_> = targets
            .into_iter()
            .filter_map(|target| self.focus_target_launcher_for_layout(target))
            .collect();

        self.deferred_focus_layout_launchers.extend(launcher_ids);
    }

    pub(super) fn flush_deferred_focus_layout(&mut self) -> Effects {
        let mut effects = Effects::None;
        for launcher_id in self.deferred_focus_layout_launchers.drain() {
            effects += DesktopEffect::RecomputeLayout(DesktopTarget::Launcher(launcher_id));
        }
        effects
    }

    /// Returns the launcher that should be re-laid-out when focus moves to/from `target`, or
    /// `None` if the target's launcher does not require focus-driven relayout.
    fn focus_target_launcher_for_layout(&self, target: &DesktopTarget) -> Option<LaunchProfileId> {
        let focused_path = self.aggregates.hierarchy.resolve_path(Some(target));
        let focused_instance = focused_path.instance()?;
        let launcher_id = self.instance_launcher(focused_instance)?;
        let instance_count = self
            .aggregates
            .hierarchy
            .get_nested(&DesktopTarget::Launcher(launcher_id))
            .len();

        self.aggregates
            .launchers
            .get(&launcher_id)
            .filter(|launcher| launcher.should_relayout_on_focus_change(instance_count))
            .map(|_| launcher_id)
    }

    pub(super) fn sync_hover_rect_to_pointer_path(
        &mut self,
        pointer_focus: Option<&DesktopTarget>,
    ) {
        let hover_placement = match pointer_focus {
            Some(DesktopTarget::Instance(instance_id)) => {
                self.instance_hover_placement(*instance_id)
            }
            Some(DesktopTarget::View(view_id)) => match self
                .aggregates
                .hierarchy
                .parent(&DesktopTarget::View(*view_id))
            {
                Some(DesktopTarget::Instance(instance_id)) => {
                    self.instance_hover_placement(*instance_id)
                }
                Some(_) => panic!("Internal error: View parent is not an instance"),
                None => None,
            },
            Some(DesktopTarget::Launcher(launcher_id)) => {
                self.placement(&DesktopTarget::Launcher(*launcher_id))
            }
            _ => None,
        };

        self.aggregates
            .project_presenter
            .set_hover_placement(hover_placement);
    }

    fn update_launcher_visor_expansion(
        &mut self,
        focused_target: Option<&DesktopTarget>,
        animate: bool,
    ) {
        let focused_path = self.aggregates.hierarchy.resolve_path(focused_target);
        let launcher_ids: Vec<_> = self.aggregates.launchers.keys().copied().collect();

        for launcher_id in launcher_ids {
            let launcher_target = DesktopTarget::Launcher(launcher_id);
            let expanded = focused_path.contains(&launcher_target);

            let launcher = self
                .aggregates
                .launchers
                .get_mut(&launcher_id)
                .expect("Launcher missing");
            launcher.set_visor_expansion(expanded, animate);
        }
    }

    fn instance_hover_placement(&self, instance_id: InstanceId) -> Option<Placement<Transform, 2>> {
        let mut placement = self.placement(&DesktopTarget::Instance(instance_id))?;
        placement.transform = self.aggregates.instances[&instance_id]
            .layout_transform_animation
            .value();
        Some(placement)
    }
}
