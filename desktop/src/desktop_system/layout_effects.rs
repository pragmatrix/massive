use anyhow::Result;

use log::error;
use massive_animation::{AnimationContext, Interpolation};
use massive_geometry::{SizePx, Transform};
use massive_layout::LayoutTopology;

use super::effects::{DesktopEffect, DesktopEffectScheduler, Effects};
use super::layout_state::PlacementUpdate;
use super::{DesktopLayoutAlgorithm, DesktopSystem, DesktopTarget, TransactionEffectsMode};
use crate::instance_presenter::STRUCTURAL_ANIMATION_DURATION;

impl DesktopSystem {
    pub(super) fn run_effects_to_completion(
        &mut self,
        context: &impl AnimationContext,
        effects_mode: TransactionEffectsMode,
        initial_effects: Effects,
    ) -> Result<()> {
        let mut effects = DesktopEffectScheduler::new(initial_effects);

        while let Some(effect) = effects.pop_next() {
            let follow_up = self.handle_effect(context, effect, effects_mode)?;
            effects.enqueue_all(follow_up);
        }

        Ok(())
    }

    fn handle_effect(
        &mut self,
        context: &impl AnimationContext,
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
                self.update_camera_effect(context, effects_mode);
                Ok(Effects::None)
            }
        }
    }

    pub(super) fn apply_focused_view_window_state(&self) -> Result<()> {
        let state = self.focused_view_window_state()?.unwrap_or_default();
        self.window
            .set_title(&self.focused_window_title(state.title)?);
        self.window.set_cursor(state.cursor);
        // Pointer-feedback state drives cursor visibility (hidden during keyboard navigation).
        self.window.set_cursor_visible(self.is_cursor_visible());
        Ok(())
    }

    fn focused_window_title(&self, terminal_title: String) -> Result<String> {
        let focused = self.event_router.keyboard_focus();
        let launcher = focused
            .and_then(|target| self.aggregates.hierarchy.launcher_of_target(target))
            .map(|id| {
                self.aggregates
                    .launchers
                    .get(&id)
                    .map(|launcher| launcher.name())
                    .expect("Focused launcher has no presenter")
            });
        let project = focused
            .and_then(|target| self.aggregates.hierarchy.project_of_target(target))
            .map(|id| {
                self.aggregates
                    .projects
                    .get(&id)
                    .map(|project| project.name())
                    .expect("Focused project has no presenter")
            });

        let mut title = if terminal_title.is_empty() {
            self.env.primary_application.clone()
        } else {
            terminal_title
        };
        for name in launcher.into_iter().chain(project) {
            title.push_str(" - ");
            title.push_str(name);
        }
        Ok(title)
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
    /// dedupes the payload-less `UpdateCamera` into a single `PostLayout` run per transaction. Pure
    /// focus changes that move no layout are handled by `transact`, which observes the focus change
    /// and emits `UpdateCamera` directly.
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

        Effects::from(DesktopEffect::UpdateCamera)
    }

    fn update_camera_effect(
        &mut self,
        context: &impl AnimationContext,
        effects_mode: TransactionEffectsMode,
    ) {
        if !effects_mode.permit_camera_moves() {
            return;
        }

        let Some(focused) = self.event_router.keyboard_focus() else {
            // Not sure what we do if nothing is focused yet.
            error!("Updating camera without something focused");
            return;
        };

        // Hmm, I think there can't be a None case here.
        let camera_target =
            self.resolve_camera_for_target_or_ancestor(focused, self.user_state.focus_depth);

        if let Some(camera) = camera_target {
            if effects_mode.animate() {
                self.camera.animate_if_changed(
                    context,
                    camera,
                    STRUCTURAL_ANIMATION_DURATION,
                    Interpolation::CubicOut,
                );
            } else {
                self.camera.set_immediately(camera);
            }
        }
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
}
