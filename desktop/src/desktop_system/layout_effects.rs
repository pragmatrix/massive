use std::collections::HashSet;

use anyhow::Result;

use massive_animation::Interpolation;
use massive_applications::InstanceId;
use massive_geometry::{PointPx, Rect, RectPx};
use massive_layout::Rect as LayoutRect;

use super::{DesktopLayoutAlgorithm, DesktopSystem, DesktopTarget, TransactionEffectsMode};
use crate::focus_path::PathResolver;
use crate::instance_presenter::STRUCTURAL_ANIMATION_DURATION;
use crate::projects::{LaunchProfileId, LauncherInstanceLayoutInput, LauncherInstanceLayoutTarget};

impl DesktopSystem {
    /// Update layout changes and the camera position.
    pub fn update_effects(&mut self, mode: Option<TransactionEffectsMode>) -> Result<()> {
        let (animate, permit_camera_moves) = match mode {
            Some(TransactionEffectsMode::Setup) => (false, true),
            Some(TransactionEffectsMode::CameraLocked) => (true, false),
            None => (true, true),
        };

        // Layout & apply rects.
        let algorithm = DesktopLayoutAlgorithm {
            aggregates: &self.aggregates,
            default_panel_size: self.default_panel_size,
        };
        let changed = self
            .layouter
            .recompute(&self.aggregates.hierarchy, &algorithm, PointPx::origin())
            .changed;
        self.apply_layout_changes(changed, animate);

        let from_focus = self.last_effects_focus.take();
        let to_focus = self.event_router.focused().cloned();
        self.apply_launcher_layout_for_focus_change(from_focus, to_focus.clone(), animate);
        self.last_effects_focus = to_focus;

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
        changed: Vec<(DesktopTarget, LayoutRect<2>)>,
        animate: bool,
    ) {
        let mut launchers_to_relayout: HashSet<LaunchProfileId> = HashSet::new();

        for (id, layout_rect) in changed {
            let rect_px: RectPx = layout_rect.into();
            let rect: Rect = rect_px.into();

            match id {
                DesktopTarget::Desktop => {}
                DesktopTarget::Instance(instance_id) => {
                    if let Some(launcher_id) = self.instance_launcher(instance_id) {
                        launchers_to_relayout.insert(launcher_id);
                    }
                }
                DesktopTarget::Group(group_id) => {
                    self.aggregates
                        .groups
                        .get_mut(&group_id)
                        .expect("Missing group")
                        .rect = rect;
                }
                DesktopTarget::Launcher(launcher_id) => {
                    launchers_to_relayout.insert(launcher_id);

                    self.aggregates
                        .launchers
                        .get_mut(&launcher_id)
                        .expect("Launcher missing")
                        .set_rect(rect, animate);
                }
                DesktopTarget::View(..) => {
                    // Robustness: Support resize here?
                }
            }
        }

        for launcher_id in launchers_to_relayout {
            self.apply_launcher_instance_layout(launcher_id, animate);
        }
    }

    fn instance_launcher(&self, instance_id: InstanceId) -> Option<LaunchProfileId> {
        let instance_target = DesktopTarget::Instance(instance_id);
        match self.aggregates.hierarchy.parent(&instance_target) {
            Some(DesktopTarget::Launcher(id)) => Some(*id),
            _ => None,
        }
    }

    fn apply_launcher_instance_layout(&mut self, launcher_id: LaunchProfileId, animate: bool) {
        let launcher_target = DesktopTarget::Launcher(launcher_id);
        let instance_inputs: Vec<LauncherInstanceLayoutInput> = self
            .aggregates
            .hierarchy
            .get_nested(&launcher_target)
            .iter()
            .filter_map(|target| match target {
                DesktopTarget::Instance(instance_id) => {
                    let instance_target = DesktopTarget::Instance(*instance_id);
                    let rect_px: RectPx =
                        (*self.layouter.rect(&instance_target).unwrap_or_else(|| {
                            panic!("Internal error: Missing layout rect for {instance_target:?}")
                        }))
                        .into();

                    Some(LauncherInstanceLayoutInput {
                        instance_id: *instance_id,
                        rect: rect_px,
                    })
                }
                _ => None,
            })
            .collect();

        let focused_instance = self
            .aggregates
            .hierarchy
            .resolve_path(self.event_router.focused())
            .instance();
        let layouts: Vec<LauncherInstanceLayoutTarget> = self
            .aggregates
            .launchers
            .get(&launcher_id)
            .expect("Launcher missing")
            .compute_instance_layout_targets(&instance_inputs, focused_instance);

        // Apply transform updates so presenter animations can interpolate to the new cylinder state.
        for layout in layouts {
            self.aggregates
                .instances
                .get_mut(&layout.instance_id)
                .expect("Instance missing")
                .set_layout(layout.rect, layout.layout_transform, animate);
        }
    }

    /// Recomputes instance layout transforms for launchers impacted by a keyboard focus change.
    ///
    /// This does not change panel geometry; it only updates per-instance layout targets
    /// (for example visor cylinder yaw/translation) by rerunning
    /// [`Self::apply_launcher_instance_layout`] for affected launchers.
    ///
    /// Behavior:
    /// - If `from == to`, this is a no-op.
    /// - Only launchers that own either the old or new focus target are considered.
    /// - A launcher is updated only when `should_relayout_on_focus_change` says its current
    ///   mode/instance-count requires focus-driven relayout.
    pub(super) fn apply_launcher_layout_for_focus_change(
        &mut self,
        from: Option<DesktopTarget>,
        to: Option<DesktopTarget>,
        animate: bool,
    ) {
        // Architecture: I don't like this before/after focus comparison test.
        // No focus transition means there is no cylinder rotation target change.
        if from == to {
            return;
        }

        // Update at most the launchers touched by the old/new focus targets.
        let mut launchers_to_update: HashSet<LaunchProfileId> = HashSet::new();
        for target in [from.as_ref(), to.as_ref()] {
            if let Some(launcher_id) = self.focus_target_launcher_for_layout(target) {
                launchers_to_update.insert(launcher_id);
            }
        }

        // Recompute launcher transforms immediately so the focus move animates right away.
        for launcher_id in launchers_to_update {
            self.apply_launcher_instance_layout(launcher_id, animate);
        }
    }

    fn focus_target_launcher_for_layout(
        &self,
        target: Option<&DesktopTarget>,
    ) -> Option<LaunchProfileId> {
        // Resolve from any focus target (instance/view/etc.) to its owning instance.
        let target = target?;
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

    fn sync_hover_rect_to_pointer_path(
        &mut self,
        pointer_focus: Option<&DesktopTarget>,
    ) {
        let hover_rect = match pointer_focus {
            Some(DesktopTarget::Instance(instance_id)) => {
                self.rect(&DesktopTarget::Instance(*instance_id))
            }
            Some(DesktopTarget::View(view_id)) => match self
                .aggregates
                .hierarchy
                .parent(&DesktopTarget::View(*view_id))
            {
                Some(DesktopTarget::Instance(instance_id)) => {
                    self.rect(&DesktopTarget::Instance(*instance_id))
                }
                Some(_) => panic!("Internal error: View parent is not an instance"),
                None => None,
            },
            Some(DesktopTarget::Launcher(launcher_id)) => {
                self.rect(&DesktopTarget::Launcher(*launcher_id))
            }
            _ => None,
        };

        self.aggregates.project_presenter.set_hover_rect(hover_rect);
    }
}
