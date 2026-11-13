//! Handles the effects of a running transaction: dispatches each effect to its handler, and holds
//! the handlers that are not about layout (ADR 0043).

use anyhow::Result;

use super::change::DesktopSystemEffect;
use super::effects::DesktopEffect;
use super::{DesktopSystem, DesktopTarget, TransactionContext, TransactionEffectsMode};
use crate::instance_manager::InstanceManager;
use crate::projects::ProjectId;

impl DesktopSystem {
    /// Handles one effect of the running transaction (ADR 0043). The handling marks the effects
    /// that read what it changed, and appends what the host has to do to `system_effects`.
    pub(super) fn handle_effect(
        &mut self,
        effect: DesktopEffect,
        context: &TransactionContext,
        instance_manager: &InstanceManager,
        system_effects: &mut Vec<DesktopSystemEffect>,
    ) -> Result<()> {
        match effect {
            DesktopEffect::RefreshColumnCount(project) => self.refresh_column_count_effect(project),
            DesktopEffect::RefreshMcpIndicators(project) => {
                self.refresh_mcp_indicators_effect(project)
            }
            DesktopEffect::CollapseSelection => self.collapse_selection_unless_set(),
            DesktopEffect::ShowZoomLevel => self.show_zoom_level_effect(context.effects_mode),
            DesktopEffect::Measure(target) => self.layout_pass().measure(target),
            DesktopEffect::Place(root) => self.layout_pass().place(root),
            DesktopEffect::Reparent(target) => self.layout_pass().reparent(target),
            DesktopEffect::ApplyLayout(target) => {
                self.layout_pass()
                    .apply(target, context.animate_layout, instance_manager)?
            }
            DesktopEffect::UpdateDesiredCamera => {
                let desired = self.camera_framing().desired_camera();
                self.camera.set_desired(desired);
            }
            DesktopEffect::SynchronizeCamera => self.camera.synchronize(context.camera_mode),
            DesktopEffect::LayoutZoomLevelIndicator => self
                .zoom_level_indicator
                .sync_layout(self.window_state.inner_size),
            DesktopEffect::UpdateHover => self
                .desktop_presenter
                .set_hover_placement(self.hover_placement()),
            DesktopEffect::SetWindowFullScreen => {
                let full_screen = self
                    .transaction
                    .window_full_screen
                    .expect("the change that marks SetWindowFullScreen records the state");
                system_effects.push(DesktopSystemEffect::SetWindowFullScreen(full_screen));
            }
            // The roots of one transaction reach the host as one set, so that it closes their
            // connections together.
            DesktopEffect::AnnounceMcpRoot(project) => match system_effects.last_mut() {
                Some(DesktopSystemEffect::McpRootsChanged(roots)) => *roots += project,
                _ => system_effects.push(DesktopSystemEffect::McpRootsChanged(project.into())),
            },
            // A setup transaction replays configuration already on disk and never persists it
            // (ADR 0013).
            DesktopEffect::PersistConfiguration => {
                if context.effects_mode != TransactionEffectsMode::Setup {
                    system_effects.push(DesktopSystemEffect::PersistConfiguration);
                }
            }
        }
        Ok(())
    }

    /// Shows the zoom level indicator for the focused target, except in setup transactions, which
    /// replay state the user did not just change.
    fn show_zoom_level_effect(&mut self, effects_mode: TransactionEffectsMode) {
        if effects_mode != TransactionEffectsMode::Setup
            && let Some((zoom_level, project_depth)) = self.zoom_navigation().focused_zoom_level()
        {
            self.zoom_level_indicator.show(zoom_level, project_depth);
        }
    }

    /// A focus or zoom change, and a configuration change the selection does not survive, collapse
    /// the selection, unless the transaction set it, because then the operation that set it
    /// planned these changes (ADR 0025, ADR 0033).
    fn collapse_selection_unless_set(&mut self) {
        if !self.transaction.selection_set {
            self.selection.collapse();
        }
    }

    /// Refreshes the shared column counts around `project`, whose slots changed, and measures the
    /// matrices whose count changed. The topology already holds the transaction's changes, so it
    /// names the project's current parent. A project the transaction removed loses its counts,
    /// and the refresh of the project that hosted it updates its former siblings (ADR 0035).
    fn refresh_column_count_effect(&mut self, project: ProjectId) {
        let aggregates = &mut self.aggregates;
        if aggregates.configuration.project(project).is_none() {
            aggregates.shared_column_counts.forget(project);
            return;
        }
        let parent = aggregates.hierarchy.parent_project_of(project);
        assert!(
            parent.is_some() || project == ProjectId::ROOT,
            "every configured project but the root sits in a matrix of the topology"
        );
        aggregates
            .shared_column_counts
            .refresh(project, parent, &aggregates.configuration)
            .into_iter()
            .for_each(|project| DesktopEffect::Measure(DesktopTarget::matrix(project)).mark());
    }
}
