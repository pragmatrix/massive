use anyhow::Result;

use massive_applications::ViewEvent;
use massive_geometry::{PixelCamera, SizePx, SizedTransform};
use massive_layout::LayoutTopology;

use super::effects::{DesktopEffect, DesktopEffectScheduler, Effects};
use super::layout_state::PlacementUpdate;
use super::{DesktopLayoutAlgorithm, DesktopSystem, DesktopTarget, TransactionEffectsMode};
use crate::instance_manager::InstanceManager;
use crate::window_state::WindowPresentationState;

impl DesktopSystem {
    pub(super) fn run_effects_to_completion(
        &mut self,
        effects_mode: TransactionEffectsMode,
        initial_effects: Effects,
        window_size: SizePx,
        instance_manager: &InstanceManager,
    ) -> Result<()> {
        let mut effects = DesktopEffectScheduler::new(initial_effects);

        while let Some(effect) = effects.pop_next() {
            let follow_up =
                self.handle_effect(effect, effects_mode, window_size, instance_manager)?;
            effects.enqueue_all(follow_up);
        }

        Ok(())
    }

    fn handle_effect(
        &mut self,
        effect: DesktopEffect,
        effects_mode: TransactionEffectsMode,
        window_size: SizePx,
        instance_manager: &InstanceManager,
    ) -> Result<Effects> {
        match effect {
            DesktopEffect::Measure(target) => self.measure_layout_effect(target, window_size),
            DesktopEffect::Place(root) => self.place_layout_effect(root, window_size),
            DesktopEffect::ApplyLayout(target) => {
                self.apply_layout_effect(target, effects_mode, instance_manager)
            }
        }
    }

    pub fn window_presentation_state(&self) -> Result<WindowPresentationState> {
        let view_window_state = self.focused_view_window_state()?.unwrap_or_default();
        let title = self.window_title(view_window_state.title);
        let cursor = view_window_state.cursor;
        // Pointer-feedback state drives cursor visibility (hidden during keyboard navigation).
        let cursor_visible = self.event_router.pointer_focus().is_some();
        Ok(WindowPresentationState {
            title,
            cursor_visible,
            cursor,
        })
    }

    fn window_title(&self, terminal_title: String) -> String {
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
        title
    }

    /// Measures one layout target in a bottom-up pass and schedules follow-up work.
    ///
    /// If any direct child is still unmeasured, this does not measure the target yet.
    /// Instead, it enqueues `Measure` for each missing child and returns immediately.
    ///
    /// Once all children are measured, this measures `target`, always schedules `Place(target)`,
    /// and re-enqueues `Measure(parent)` only when the measured size changed.
    fn measure_layout_effect(
        &mut self,
        target: DesktopTarget,
        window_size: SizePx,
    ) -> Result<Effects> {
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
            focus_depth: self.focus_depth,
            window_size,
        };

        let outcome =
            self.layout_state
                .measure_node(&target, &self.aggregates.hierarchy, &algorithm);

        let mut effects = [DesktopEffect::Place(target)].into();
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
    fn place_layout_effect(&mut self, root: DesktopTarget, window_size: SizePx) -> Result<Effects> {
        let focused_instance = self.focused_path().instance();
        let algorithm = DesktopLayoutAlgorithm {
            aggregates: &self.aggregates,
            default_panel_size: self.default_panel_size,
            focused_instance,
            focus_depth: self.focus_depth,
            window_size,
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

    /// Applies one target's local placement to the renderer.
    ///
    fn apply_layout_effect(
        &mut self,
        target: DesktopTarget,
        effects_mode: TransactionEffectsMode,
        instance_manager: &InstanceManager,
    ) -> Result<Effects> {
        let placement = self.layout_state.local_placement(&target);
        let layout_size = placement.rect.size;
        let size_px = SizePx::new(layout_size[0], layout_size[1]);
        let layout = SizedTransform::new(size_px, placement.transform);
        self.apply_layout(
            target,
            layout,
            placement.visible,
            effects_mode.permit_animations(),
            instance_manager,
        )?;

        Ok(Effects::None)
    }

    fn apply_layout(
        &mut self,
        target: DesktopTarget,
        layout: SizedTransform,
        visible: bool,
        animate: bool,
        instance_manager: &InstanceManager,
    ) -> Result<()> {
        match target {
            DesktopTarget::Desktop => {}
            DesktopTarget::Instance(instance_id) => {
                self.aggregates
                    .instances
                    .get_mut(&instance_id)
                    .expect("Instance missing")
                    .set_layout(layout, visible, animate);
            }
            DesktopTarget::Project(project_id) => {
                self.aggregates
                    .projects
                    .get_mut(&project_id)
                    .expect("Missing project")
                    .set_layout(layout);
            }
            DesktopTarget::ProjectHeader(project_id) => {
                self.aggregates
                    .projects
                    .get_mut(&project_id)
                    .expect("Missing project")
                    .header
                    .set_layout(layout, animate);
            }
            DesktopTarget::ProjectMatrix(project_id) => {
                self.aggregates
                    .projects
                    .get_mut(&project_id)
                    .expect("Missing project")
                    .matrix
                    .set_layout(layout);
            }
            DesktopTarget::Launcher(launcher_id) => {
                self.aggregates
                    .launchers
                    .get_mut(&launcher_id)
                    .expect("Launcher missing")
                    .set_layout(layout, animate);
            }
            DesktopTarget::View(view_id) => {
                let Some(instance_id) = self.aggregates.hierarchy.instance_of_target(&target)
                else {
                    return Ok(());
                };
                if let Some(instance) = self.aggregates.instances.get_mut(&instance_id)
                    && let Some(resized) = instance.set_view_layout(view_id, layout)?
                {
                    instance_manager
                        .send_view_event((instance_id, view_id), ViewEvent::Resized(resized))?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn resolve_desired_camera(&self, window_size: SizePx) -> Option<PixelCamera> {
        let focused = self.event_router.keyboard_focus()?;
        Some(self.resolve_camera_for_target_or_ancestor(focused, self.focus_depth, window_size))
    }
}
