use anyhow::Result;

use massive_animation::Interpolation;
use massive_applications::InstanceId;
use massive_geometry::{Point, SizePx, Transform};
use massive_layout::{LayoutTopology, Placement, Size as LayoutSize};

use super::effects::{DesktopEffect, DesktopEffectScheduler, Effects};
use super::layout_state::PlacementUpdate;
use super::{
    DesktopLayoutAlgorithm, DesktopSystem, DesktopTarget, TransactionEffectsMode, UserState,
};
use crate::instance_presenter::STRUCTURAL_ANIMATION_DURATION;
use crate::projects::LaunchProfileId;

impl DesktopSystem {
    pub(super) fn run_effects_to_completion(
        &mut self,
        effects_mode: TransactionEffectsMode,
        initial_effects: Effects,
    ) -> Result<()> {
        if !effects_mode.permit_camera_moves() {
            // Lock camera motion immediately, including already running camera animations.
            // Ergonomics: There should probably be a function for that in Animated.
            let camera = *self.camera.value();
            self.camera.set_immediately(camera);
        }

        let mut effects = DesktopEffectScheduler::new(initial_effects);

        while let Some(effect) = effects.pop_next() {
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
            DesktopEffect::Measure(target) => self.measure_layout_effect(target),
            DesktopEffect::Place(root) => self.place_layout_effect(root),
            DesktopEffect::ApplyLayout(target) => {
                Ok(self.apply_layout_effect(target, effects_mode))
            }
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

    pub(super) fn apply_focused_view_window_state(&self) -> Result<()> {
        let state = self.focused_view_window_state()?.unwrap_or_default();
        self.window.set_title(&state.title);
        self.window.set_cursor(state.cursor);
        // Pointer-feedback state drives cursor visibility (hidden during keyboard navigation).
        self.window.set_cursor_visible(self.is_cursor_visible());
        Ok(())
    }

    /// Measures one layout target in a bottom-up pass and schedules follow-up work.
    ///
    /// If any direct child is still unmeasured, this does not measure the target yet.
    /// Instead, it enqueues `Measure` for each missing child and returns immediately.
    ///
    /// Once all children are measured, this measures `target`, always schedules `Place(target)`,
    /// and re-enqueues `Measure(parent)` only when the measured size changed.
    fn measure_layout_effect(&mut self, target: DesktopTarget) -> Result<Effects> {
        // If measurements of children are not available, push them as effects and return early.
        let missing_children = self
            .layout_state
            .missing_child_measures(&target, &self.aggregates.hierarchy);
        if !missing_children.is_empty() {
            let mut effects = Effects::None;
            for child in missing_children {
                effects += DesktopEffect::Measure(child);
            }
            return Ok(effects);
        }

        let focused_instance = self.focused_path().instance();
        let algorithm = DesktopLayoutAlgorithm {
            aggregates: &self.aggregates,
            default_panel_size: self.default_panel_size,
            focused_instance,
        };

        let outcome =
            self.layout_state
                .measure_node(&target, &self.aggregates.hierarchy, &algorithm);

        let mut effects = Effects::from(DesktopEffect::Place(target));
        if outcome.size_changed
            && let Some(parent) = outcome.parent
        {
            effects += DesktopEffect::Measure(parent);
        }

        Ok(effects)
    }

    /// Places direct children under `root` and schedules render-facing updates.
    ///
    /// This consumes measured child sizes from layout state, computes child placements, and
    /// updates the local placement cache. It emits `ApplyLayout` only for targets whose local
    /// placement changed; camera and hover synchronization follow from `ApplyLayout` itself.
    fn place_layout_effect(&mut self, root: DesktopTarget) -> Result<Effects> {
        let focused_instance = self.focused_path().instance();
        let algorithm = DesktopLayoutAlgorithm {
            aggregates: &self.aggregates,
            default_panel_size: self.default_panel_size,
            focused_instance,
        };

        let children = self.aggregates.hierarchy.children_of(&root);
        let placement_outcomes = self
            .layout_state
            .place_children_of(&root, children, &algorithm);

        // `Place(root)` computes each child's local placement here, so `ApplyLayout(child)` is what
        // pushes that placement to the renderer. `Place(child)` only re-places the child's own
        // descendants and never applies the child's own placement, so it cannot stand in for
        // `ApplyLayout(child)`.
        let mut effects = Effects::None;
        for (child, outcome) in children.iter().zip(placement_outcomes) {
            match outcome {
                PlacementUpdate::Unchanged => {}
                PlacementUpdate::ChangedSizeUnchanged => {
                    // Placement changed but size did not, so descendants stay valid; apply only.
                    effects += DesktopEffect::ApplyLayout(child.clone());
                }
                PlacementUpdate::ChangedSizeChanged => {
                    // Size changed, so re-place descendants against the new size, then apply the
                    // child's own newly computed placement.
                    effects += DesktopEffect::Place(child.clone());
                    effects += DesktopEffect::ApplyLayout(child.clone());
                }
            }
        }

        Ok(effects)
    }

    /// Applies one target's local placement to the renderer and refreshes camera and hover.
    ///
    /// Camera and hover follow from the placement being applied, so they are scheduled here rather
    /// than per `Place` pass: this runs only when a placement actually changed, and the scheduler
    /// dedupes the payload-less `UpdateCamera`/`SyncHover` into a single `PostLayout` run per
    /// transaction. Pure focus changes that move no layout emit `UpdateCamera` directly via
    /// `apply_keyboard_focus_change_effects`.
    fn apply_layout_effect(
        &mut self,
        target: DesktopTarget,
        effects_mode: TransactionEffectsMode,
    ) -> Effects {
        let placement = self.layout_state.local_placement(&target);
        let layout_size = placement.rect.size;
        let size_px = SizePx::new(layout_size[0], layout_size[1]);
        self.apply_layout(
            target,
            size_px,
            placement.transform,
            placement.visible,
            effects_mode.animate(),
        );

        Effects::from(DesktopEffect::UpdateCamera) + DesktopEffect::SyncHover.into()
    }

    fn update_camera_effect(&mut self, effects_mode: TransactionEffectsMode) {
        if !effects_mode.permit_camera_moves() {
            return;
        }

        let camera_target = match &self.user_state {
            UserState::Focused => self
                .event_router
                .focused()
                .and_then(|target| self.camera_for_focus(target)),
            UserState::Overview(target) => self.camera_for_overview_target(target),
        };

        if let Some(camera) = camera_target {
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
        visible: bool,
        animate: bool,
    ) {
        match target {
            DesktopTarget::Desktop => {}
            DesktopTarget::Instance(instance_id) => {
                self.aggregates
                    .instances
                    .get_mut(&instance_id)
                    .expect("Instance missing")
                    .set_layout(size_px, transform, visible, animate);
            }
            DesktopTarget::Project(project_id) => {
                self.aggregates
                    .projects
                    .get_mut(&project_id)
                    .expect("Missing project")
                    .set_layout(size_px, transform);
            }
            DesktopTarget::ProjectHeader(project_id) => {
                self.aggregates
                    .projects
                    .get_mut(&project_id)
                    .expect("Missing project")
                    .header
                    .set_layout(size_px, transform, animate);
            }
            DesktopTarget::ProjectMatrix(project_id) => {
                self.aggregates
                    .projects
                    .get_mut(&project_id)
                    .expect("Missing project")
                    .matrix
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

    pub(super) fn instance_launcher(&self, instance_id: InstanceId) -> Option<LaunchProfileId> {
        let instance_target = DesktopTarget::Instance(instance_id);
        match self.aggregates.hierarchy.parent(&instance_target) {
            Some(DesktopTarget::Launcher(id)) => Some(*id),
            _ => None,
        }
    }

    pub(super) fn sync_hover_rect_to_pointer_path(
        &mut self,
        pointer_focus: Option<&DesktopTarget>,
    ) {
        let hover_placement = match pointer_focus {
            Some(DesktopTarget::Instance(instance_id)) => {
                Some(self.instance_hover_placement(*instance_id))
            }
            Some(DesktopTarget::View(view_id)) => match self
                .aggregates
                .hierarchy
                .parent(&DesktopTarget::View(*view_id))
            {
                Some(DesktopTarget::Instance(instance_id)) => {
                    Some(self.instance_hover_placement(*instance_id))
                }
                Some(_) => panic!("Internal error: View parent is not an instance"),
                None => None,
            },
            Some(DesktopTarget::Launcher(launcher_id)) => {
                Some(self.placement(&DesktopTarget::Launcher(*launcher_id)))
            }
            _ => None,
        };

        self.desktop_presenter.set_hover_placement(hover_placement);
    }

    fn instance_hover_placement(&self, instance_id: InstanceId) -> Placement<Transform, 2> {
        let mut placement = self.placement(&DesktopTarget::Instance(instance_id));

        // Keep hover aligned with animated instance motion by composing the current instance-local
        // animated transform with the launcher's world transform.
        let Some(instance_presenter) = self.aggregates.instances.get(&instance_id) else {
            return placement;
        };
        let Some(launcher_id) = self.instance_launcher(instance_id) else {
            return placement;
        };
        let launcher_placement = self.placement(&DesktopTarget::Launcher(launcher_id));

        let launcher_anchor = Self::layout_center(launcher_placement.rect.size);
        let instance_anchor = Self::layout_center(placement.rect.size);
        placement.transform = Transform::compose_with_anchor(
            launcher_placement.transform,
            launcher_anchor,
            *instance_presenter.layout_transform_animation.latest_value(),
            instance_anchor,
        );

        placement
    }

    fn layout_center(size: LayoutSize<2>) -> Point {
        Point::new(size[0] as f64 * 0.5, size[1] as f64 * 0.5)
    }
}
