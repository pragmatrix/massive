use anyhow::Result;

use massive_animation::Interpolation;
use massive_applications::InstanceId;
use massive_geometry::{PointPx, SizePx, Transform};
use massive_layout::Placement;

use super::{DesktopLayoutAlgorithm, DesktopSystem, DesktopTarget, TransactionEffectsMode};
use crate::focus_path::PathResolver;
use crate::instance_presenter::STRUCTURAL_ANIMATION_DURATION;
use crate::projects::LaunchProfileId;

impl DesktopSystem {
    /// Update layout changes and the camera position.
    pub fn update_effects(&mut self, mode: Option<TransactionEffectsMode>) -> Result<()> {
        let (animate, permit_camera_moves) = match mode {
            Some(TransactionEffectsMode::Setup) => (false, true),
            Some(TransactionEffectsMode::CameraLocked) => (true, false),
            None => (true, true),
        };

        if matches!(mode, Some(TransactionEffectsMode::CameraLocked)) {
            // Lock camera motion immediately, including already running camera animations.
            let camera = self.camera.value();
            self.camera.set_immediately(camera);
        }

        // Layout & apply rects + transforms.
        let focused_instance = self
            .aggregates
            .hierarchy
            .resolve_path(self.event_router.focused())
            .instance();
        let algorithm = DesktopLayoutAlgorithm {
            aggregates: &self.aggregates,
            default_panel_size: self.default_panel_size,
            focused_instance,
        };
        let changed = self
            .layouter
            .recompute(&self.aggregates.hierarchy, &algorithm, PointPx::origin())
            .changed;
        self.apply_layout_changes(changed, animate);

        // Camera

        if permit_camera_moves && let Some(focused) = self.event_router.focused() {
            let camera = self.camera_for_focus(focused);
            if let Some(camera) = camera {
                if animate {
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

        // Hover

        let pointer_focus = if self.pointer_feedback_enabled {
            self.event_router.pointer_focus().cloned()
        } else {
            None
        };
        self.sync_hover_rect_to_pointer_path(pointer_focus.as_ref());

        Ok(())
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
    ) {
        for target in targets {
            if let Some(launcher_id) = self.focus_target_launcher_for_layout(target) {
                self.layouter
                    .mark_reflow_pending(DesktopTarget::Launcher(launcher_id));
            }
        }
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

    fn sync_hover_rect_to_pointer_path(&mut self, pointer_focus: Option<&DesktopTarget>) {
        let hover_placement = match pointer_focus {
            Some(DesktopTarget::Instance(instance_id)) => {
                self.placement(&DesktopTarget::Instance(*instance_id))
            }
            Some(DesktopTarget::View(view_id)) => match self
                .aggregates
                .hierarchy
                .parent(&DesktopTarget::View(*view_id))
            {
                Some(DesktopTarget::Instance(instance_id)) => {
                    self.placement(&DesktopTarget::Instance(*instance_id))
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
}
