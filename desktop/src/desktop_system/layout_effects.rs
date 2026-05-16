use anyhow::Result;

use massive_animation::Interpolation;
use massive_applications::InstanceId;
use massive_geometry::{Point, SizePx, Transform};
use massive_layout::{Placement, Size as LayoutSize};

use super::effects::{DesktopEffect, DesktopEffectQueue, Effects};
use super::{DesktopLayoutAlgorithm, DesktopSystem, DesktopTarget, TransactionEffectsMode};
use crate::focus_path::PathResolver;
use crate::instance_presenter::STRUCTURAL_ANIMATION_DURATION;
use crate::projects::LaunchProfileId;

impl DesktopSystem {
    pub(super) fn transaction_effects(&self, command_effects: Effects) -> Effects {
        let mut effects = Effects::from(DesktopEffect::UpdateLauncherExpansion);
        effects += command_effects;
        effects += DesktopEffect::ReflowLayout(DesktopTarget::Desktop);
        effects
    }

    pub(super) fn run_effects_to_completion(
        &mut self,
        effects_mode: TransactionEffectsMode,
        initial_effects: Effects,
    ) -> Result<()> {
        if !effects_mode.permit_camera_moves() {
            // Lock camera motion immediately, including already running camera animations.
            let camera = self.camera.value();
            self.camera.set_immediately(camera);
        }

        let mut effects = DesktopEffectQueue::default();
        effects.enqueue_all(initial_effects);

        while let Some(effect) = effects.pop_front() {
            let follow_up = self.handle_effect(effect, effects_mode)?;
            effects.enqueue_all(follow_up);
        }

        Ok(())
    }

    fn handle_effect(
        &mut self,
        effect: DesktopEffect,
        effects_mode: TransactionEffectsMode,
    ) -> Result<Effects> {
        match effect {
            DesktopEffect::UpdateLauncherExpansion => {
                self.update_launcher_expansion_effect(effects_mode);
                Ok(Effects::None)
            }
            DesktopEffect::ReflowLayout(target) => self.handle_reflow_layout_effect(target),
            DesktopEffect::PlaceNode(root) => self.place_layout_effect(root),
            DesktopEffect::ApplyLayout(target) => self.apply_layout_effect(target, effects_mode),
            DesktopEffect::UpdateCamera => {
                self.update_camera_effect(effects_mode);
                Ok(Effects::None)
            }
            DesktopEffect::SyncHover => {
                self.sync_hover_effect();
                Ok(Effects::None)
            }
        }
    }

    fn update_launcher_expansion_effect(&mut self, effects_mode: TransactionEffectsMode) {
        let focused_target = self.event_router.focused().cloned();
        self.update_launcher_visor_expansion(focused_target.as_ref(), effects_mode.animate());
    }

    fn handle_reflow_layout_effect(&mut self, target: DesktopTarget) -> Result<Effects> {
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
                effects += DesktopEffect::ReflowLayout(child);
            }
            // Redundant by current semantics: each first child measure reports size_changed and
            // re-enqueues its parent. Keep this explicit parent reflow to preserve behavior if
            // measure_node/size_changed rules change.
            effects += DesktopEffect::ReflowLayout(target);
            return Ok(effects);
        }

        let outcome =
            self.layout_state
                .measure_node(&target, &self.aggregates.hierarchy, &algorithm);

        let mut effects = Effects::from(DesktopEffect::PlaceNode(target));
        if outcome.size_changed
            && let Some(parent) = outcome.parent
        {
            effects += DesktopEffect::ReflowLayout(parent);
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

        let changed_targets =
            self.layout_state
                .place_children_of(&root, &self.aggregates.hierarchy, &algorithm);

        let mut effects = Effects::None;
        for target in changed_targets {
            effects += DesktopEffect::ApplyLayout(target);
        }
        effects += DesktopEffect::UpdateCamera;
        effects += DesktopEffect::SyncHover;

        Ok(effects)
    }

    fn apply_layout_effect(
        &mut self,
        target: DesktopTarget,
        effects_mode: TransactionEffectsMode,
    ) -> Result<Effects> {
        if let Some(placement) = self.layout_state.local_placement(&target) {
            let layout_size = placement.rect.size;
            let size_px = SizePx::new(layout_size[0], layout_size[1]);
            self.apply_layout(target, size_px, placement.transform, effects_mode.animate());
        }

        Ok(Effects::None)
    }

    fn update_camera_effect(&mut self, effects_mode: TransactionEffectsMode) {
        if !effects_mode.permit_camera_moves() {
            return;
        }

        let focused_target = self.event_router.focused().cloned();
        if let Some(focused) = focused_target.as_ref() {
            let camera = self.camera_for_focus(focused);
            if let Some(camera) = camera {
                if effects_mode.animate() {
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

    fn apply_layout(
        &mut self,
        target: DesktopTarget,
        size_px: SizePx,
        transform: Transform,
        animate: bool,
    ) {
        match target {
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
                    .set_layout(size_px, transform);
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
                effects += DesktopEffect::ReflowLayout(DesktopTarget::Launcher(launcher_id));
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
            effects += DesktopEffect::ReflowLayout(DesktopTarget::Launcher(launcher_id));
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

        // Keep hover aligned with animated instance motion by composing the current instance-local
        // animated transform with the launcher's world transform.
        let Some(instance_presenter) = self.aggregates.instances.get(&instance_id) else {
            return Some(placement);
        };
        let Some(launcher_id) = self.instance_launcher(instance_id) else {
            return Some(placement);
        };
        let Some(launcher_placement) = self.placement(&DesktopTarget::Launcher(launcher_id)) else {
            return Some(placement);
        };

        let launcher_anchor = Self::layout_center(launcher_placement.rect.size);
        let instance_anchor = Self::layout_center(placement.rect.size);
        placement.transform = Transform::compose_with_anchor(
            launcher_placement.transform,
            launcher_anchor,
            instance_presenter.layout_transform_animation.value(),
            instance_anchor,
        );

        Some(placement)
    }

    fn layout_center(size: LayoutSize<2>) -> Point {
        Point::new(size[0] as f64 * 0.5, size[1] as f64 * 0.5)
    }
}
