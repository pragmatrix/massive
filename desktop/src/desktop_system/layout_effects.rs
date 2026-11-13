use anyhow::Result;
use log::warn;

use massive_applications::{InstanceId, ViewEvent, ViewId};
use massive_geometry::{BoxPx, PixelCamera, Point, Rect, SizePx, SizedTransform};
use massive_layout::LayoutTopology;
use massive_renderer::RenderGeometry;

use super::change::DesktopSystemEffect;
use super::effects::DesktopEffect;
use super::layout_state::PlacementUpdate;
use super::navigation::ZoomGeometry;
use super::{
    Aggregates, AppliedLayout, DesktopLayoutAlgorithm, DesktopSystem, DesktopTarget,
    FocusableTarget, InstancePart, LayoutContext, TargetLayout, TransactionContext,
    TransactionEffectsMode,
};
use crate::coordinate_mapper::CoordinateMapper;
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::instance_presenter::TITLE_SEPARATOR;
use crate::projects::{ProjectId, slot_extent};
use crate::window_state::WindowPresentationState;

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
            DesktopEffect::Measure(target) => self.measure_layout_effect(target),
            DesktopEffect::Place(root) => self.place_layout_effect(root),
            DesktopEffect::Reparent(target) => self.reparent_effect(target),
            DesktopEffect::ApplyLayout(target) => {
                self.apply_layout_effect(target, context.animate_layout, instance_manager)?
            }
            DesktopEffect::UpdateDesiredCamera => {
                let desired = self.resolve_desired_camera();
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

    pub fn window_presentation_state(
        &self,
        surface_size: SizePx,
    ) -> Result<WindowPresentationState> {
        let (view_path, view_window_state) = self.focused_view_window_state()?.unzip();
        let view_window_state = view_window_state.unwrap_or_default();
        let (ime_view, input_area) =
            self.window_ime(view_path.zip(view_window_state.ime_area), surface_size);
        let title = self.window_title(view_window_state.title);
        let cursor = view_window_state.cursor;
        // Pointer-feedback state drives cursor visibility (hidden during keyboard navigation).
        let cursor_visible = self.event_router.pointer_focus().is_some();
        Ok(WindowPresentationState {
            title,
            cursor_visible,
            cursor,
            ime_view,
            input_area,
        })
    }

    /// The view the IME composes for, and where macOS places its text input UI. Only a focused
    /// view that declares an IME area gets the IME, so a composition never reaches a view that
    /// can't take text.
    fn window_ime(
        &self,
        focused_ime: Option<(ViewPath, Rect)>,
        surface_size: SizePx,
    ) -> (Option<ViewId>, Option<BoxPx>) {
        let ime_view = focused_ime.map(|(view_path, _)| view_path.view);
        let ime_target = focused_ime.map(|(view_path, area)| {
            (
                DesktopTarget::view(view_path.instance, view_path.view),
                area,
            )
        });
        (ime_view, self.input_area(ime_target, surface_size))
    }

    /// The focused view's IME area, or else the whole focused target, so the caps lock indicator
    /// follows the focus. `None` while nothing visible is focused.
    fn input_area(
        &self,
        ime: Option<(DesktopTarget, Rect)>,
        surface_size: SizePx,
    ) -> Option<BoxPx> {
        // Projected through the camera's target, so the area moves once instead of following an
        // animation; placements are layout targets already.
        let geometry = RenderGeometry::new(surface_size, *self.camera.target());
        let mapper =
            CoordinateMapper::new(&self.aggregates.hierarchy, &self.layout_state, &geometry);
        let (target, ime_area) = match ime {
            Some((target, area)) => (target, Some(area)),
            None => (self.focused_target()?, None),
        };
        let surface = mapper.surface(&target)?;
        let area = ime_area.unwrap_or_else(|| Rect::from_size(surface.size));
        let corners = area
            .to_quad()
            .into_iter()
            .map(|corner| mapper.to_surface(corner, &surface))
            .collect();
        Some(surface_bounds(corners))
    }

    fn window_title(&self, terminal_title: String) -> String {
        // Window title suffixes are configuration names, read from the aggregate, not
        // from presenters.
        let launcher = self
            .focused_target()
            .and_then(|target| self.aggregates.hierarchy.launcher_of_target(&target))
            .map(|id| {
                self.aggregates
                    .configuration
                    .launcher(id)
                    .expect("Focused launcher has no presenter")
                    .name
                    .as_str()
            });
        let project = self
            .focused_target()
            .map(|target| self.aggregates.hierarchy.project_of_target(&target))
            .map(|id| {
                self.aggregates
                    .configuration
                    .project(id)
                    .expect("Focused project has no presenter")
                    .name
                    .as_str()
            });

        let mut title = if terminal_title.is_empty() {
            self.env.primary_application.clone()
        } else {
            terminal_title
        };
        for name in launcher.into_iter().chain(project) {
            title.push_str(TITLE_SEPARATOR);
            title.push_str(name);
        }
        title
    }

    /// Measures one layout target in a bottom-up pass and marks the effects that read its result.
    ///
    /// If any direct child is still unmeasured, this does not measure the target yet. Instead, it
    /// marks `Measure` for each missing child, whose measure marks this target's again.
    ///
    /// Once all children are measured, this measures `target`, always marks `Place(target)`, and
    /// marks `Measure(parent)` only when the measured size changed.
    fn measure_layout_effect(&mut self, target: DesktopTarget) {
        let missing_children = self
            .layout_state
            .missing_child_measures(&target, &self.aggregates.hierarchy);
        if !missing_children.is_empty() {
            for child in missing_children {
                DesktopEffect::Measure(child).mark();
            }
            return;
        }

        let focused_instance = self.layout_focus;
        let algorithm = DesktopLayoutAlgorithm {
            desktop: &self.desktop_presenter,
            aggregates: &self.aggregates,
            context: self.aggregates.layout_context(
                self.default_panel_size,
                self.window_state.inner_size,
                focused_instance,
            ),
        };

        let outcome =
            self.layout_state
                .measure_node(&target, &self.aggregates.hierarchy, &algorithm);

        DesktopEffect::Place(target).mark();
        if outcome.size_changed
            && let Some(parent) = outcome.parent
        {
            DesktopEffect::Measure(parent).mark();
        }
    }

    /// Places direct children under `root` and marks render-facing updates.
    ///
    /// This consumes measured child sizes from layout state, computes child placements, and
    /// updates the local placement cache. It marks `ApplyLayout` only for targets whose local
    /// placement changed. The camera and the hover outline read placements, so they follow
    /// automatically (ADR 0043).
    ///
    /// `root`'s own placement is not this pass's to write: it belongs to `root`'s parent, and a
    /// parent re-places a child only when the child's measured size changed — so a placement
    /// dropped here (a matrix slot's rect, a presentation scale) would never be restored. Only
    /// the parentless root, which has no parent, is placed here.
    fn place_layout_effect(&mut self, root: DesktopTarget) {
        let focused_instance = self.layout_focus;
        let algorithm = DesktopLayoutAlgorithm {
            desktop: &self.desktop_presenter,
            aggregates: &self.aggregates,
            context: self.aggregates.layout_context(
                self.default_panel_size,
                self.window_state.inner_size,
                focused_instance,
            ),
        };

        let children = self.aggregates.hierarchy.children_of(&root);
        // The exception to "every target is placed by its parent": the parentless root
        // has no parent to place it.
        if self.aggregates.hierarchy.parent_of(&root).is_none() {
            self.layout_state.place_root(&root);
        }
        let placement_outcomes = self
            .layout_state
            .place_children_of(&root, children, &algorithm);

        // `Place(root)` computes each child's local placement here, so `ApplyLayout(child)` is what
        // pushes that placement to the renderer. `Place(child)` only re-places the child's own
        // descendants and never applies the child's own placement, so it cannot stand in for
        // `ApplyLayout(child)`.
        for (child, &outcome) in children.iter().zip(placement_outcomes) {
            match outcome {
                PlacementUpdate::Unchanged => {}
                PlacementUpdate::ChangedSizeUnchanged => {
                    // Placement changed but size did not, so descendants stay valid; apply only.
                    DesktopEffect::ApplyLayout(child.clone()).mark();
                }
                PlacementUpdate::ChangedSizeChanged => {
                    // Size changed, so re-place descendants against the new size, then apply the
                    // child's own newly computed placement.
                    DesktopEffect::Place(child.clone()).mark();
                    DesktopEffect::ApplyLayout(child.clone()).mark();
                }
            }
        }
    }

    /// Converts the on-screen position of a re-parented target into its new parent's coordinates,
    /// then marks the target's layout. The layout then animates from where the target is on
    /// screen. Placement may also mark `ApplyLayout` for the same target, and the scheduler merges
    /// the two.
    fn reparent_effect(&mut self, target: DesktopTarget) {
        self.reparenting
            .take_rebase(&target)
            .expect("a reparent effect follows a re-parent")
            .apply(&self.aggregates, &self.layout_state);
        DesktopEffect::ApplyLayout(target).mark();
    }

    /// Applies one target's local placement to the renderer.
    ///
    fn apply_layout_effect(
        &mut self,
        target: DesktopTarget,
        animate: bool,
        instance_manager: &InstanceManager,
    ) -> Result<()> {
        let placement = self.layout_state.local_placement(&target);
        let layout_size = placement.rect.size;
        let size_px = SizePx::new(layout_size[0], layout_size[1]);
        let layout = SizedTransform::new(size_px, placement.transform);
        self.apply_layout(target, layout, placement.visible, animate, instance_manager)
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
            DesktopTarget::Desktop => {
                let applied = self
                    .desktop_presenter
                    .apply_layout((), layout, visible, animate)?;
                assert_eq!(applied, AppliedLayout::Applied);
            }
            DesktopTarget::Instance(instance_id, part) => {
                // An instance can end while the layout loop still re-places its view;
                // event_forwarding and the ResizeAll arm tolerate the same race.
                let Some(instance) = self.aggregates.instances.get_mut(&instance_id) else {
                    assert!(matches!(part, InstancePart::View(_)), "Instance missing");
                    return Ok(());
                };
                if let AppliedLayout::ViewResized { view, size } =
                    instance.apply_layout(part, layout, visible, animate)?
                    && let Err(e) = instance_manager
                        .send_view_event((instance_id, view), ViewEvent::Resized(size))
                {
                    warn!("Sending view resize to {instance_id:?} failed with {e}");
                }
            }
            DesktopTarget::Project(project_id, part) => {
                let applied = self
                    .aggregates
                    .projects
                    .get_mut(&project_id)
                    .expect("Missing project")
                    .apply_layout(part, layout, visible, animate)?;
                assert_eq!(applied, AppliedLayout::Applied);
            }
            DesktopTarget::Launcher(launcher_id) => {
                let applied = self
                    .aggregates
                    .launchers
                    .get_mut(&launcher_id)
                    .expect("Launcher missing")
                    .apply_layout((), layout, visible, animate)?;
                assert_eq!(applied, AppliedLayout::Applied);
            }
        }
        Ok(())
    }

    pub(super) fn resolve_desired_camera(&self) -> Option<PixelCamera> {
        let focused = self.keyboard_focus.focused()?;
        let zoom_geometry = self.zoom_geometry();
        let camera = zoom_geometry.resolve_camera_for_target(focused, self.zoom_level);
        if !self.selection.is_extended() {
            return Some(camera);
        }
        let Some(selection_camera) = self.selection_camera(focused, &zoom_geometry) else {
            return Some(camera);
        };
        Some(zoom_geometry.joined_camera(camera, selection_camera))
    }

    /// The camera that frames the extended selection around the `focused` target with a margin
    /// around it (ADR 0032). `None` when the selection has no outline.
    fn selection_camera(
        &self,
        focused: FocusableTarget,
        zoom_geometry: &ZoomGeometry,
    ) -> Option<PixelCamera> {
        let context = self.aggregates.layout_context(
            self.default_panel_size,
            self.window_state.inner_size,
            self.layout_focus,
        );
        let outline = self.selection_outline(focused, &context)?;
        Some(zoom_geometry.selection_camera(&outline, slot_extent(&context)))
    }
}

impl Aggregates {
    pub(super) fn layout_context(
        &self,
        default_panel_size: SizePx,
        window_size: SizePx,
        focused_instance: Option<InstanceId>,
    ) -> LayoutContext<'_> {
        LayoutContext {
            hierarchy: &self.hierarchy,
            configuration: &self.configuration,
            shared_column_counts: &self.shared_column_counts,
            default_panel_size,
            window_size,
            focused_instance,
            title_bar_metrics: self.instance_title_bar_metrics,
        }
    }
}

/// The pixel box macOS places its text input UI next to, from a target's projected corners: a
/// rotated or perspective target doesn't project to an axis-aligned rect, so its bounding box is
/// the area. Not built with `Rect::joined`, which skips empty rects and so ignores every point.
fn surface_bounds(points: Vec<Point>) -> BoxPx {
    assert!(!points.is_empty());
    let (min, max) = points.iter().fold(
        (
            (f64::INFINITY, f64::INFINITY),
            (f64::NEG_INFINITY, f64::NEG_INFINITY),
        ),
        |((min_x, min_y), (max_x, max_y)), p| {
            (
                (min_x.min(p.x), min_y.min(p.y)),
                (max_x.max(p.x), max_y.max(p.y)),
            )
        },
    );
    BoxPx::new(
        (min.0.floor() as i32, min.1.floor() as i32).into(),
        (max.0.ceil() as i32, max.1.ceil() as i32).into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_bounds_cover_every_point() {
        let corners = [(34.5, 148.5), (48.5, 148.5), (48.5, 177.5), (34.5, 177.5)]
            .map(|(x, y)| Point::new(x, y))
            .to_vec();

        assert_eq!(
            surface_bounds(corners),
            BoxPx::new((34, 148).into(), (49, 178).into())
        );
    }
}
