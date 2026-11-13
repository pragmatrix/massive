//! The layout effects: measuring targets bottom-up, placing their children top-down, re-basing
//! re-parented targets, and applying placements to the presenters (ADR 0043).

use anyhow::Result;
use derive_more::Debug;
use log::warn;

use massive_applications::ViewEvent;
use massive_geometry::{SizePx, SizedTransform};
use massive_layout::LayoutTopology;

use super::effects::DesktopEffect;
use super::layout_state::PlacementUpdate;
use super::reparenting::Reparenting;
use super::{
    Aggregates, AppliedLayout, DesktopLayoutAlgorithm, DesktopLayoutState, DesktopSystem,
    DesktopTarget, InstancePart, LayoutContext, LayoutInputs, TargetLayout,
};
use crate::desktop_presenter::DesktopPresenter;
use crate::instance_manager::InstanceManager;

impl DesktopSystem {
    /// Lends the layout effects the parts of the desktop system they read and write.
    pub fn layout_pass(&mut self) -> LayoutPass<'_> {
        let inputs = self.layout_inputs();
        LayoutPass {
            layout_state: &mut self.layout_state,
            aggregates: &mut self.aggregates,
            desktop_presenter: &mut self.desktop_presenter,
            reparenting: &mut self.reparenting,
            inputs,
        }
    }

    /// The values layout reads beside the aggregates. Layout follows the layout focus, which
    /// catches up with the keyboard focus only in transactions that may move the camera
    /// (ADR 0002).
    pub fn layout_inputs(&self) -> LayoutInputs {
        LayoutInputs {
            default_panel_size: self.default_panel_size,
            window_size: self.window_state.inner_size,
            focused_instance: self.layout_focus,
        }
    }
}

/// What the layout effects work on: the layout state, the presenters they apply placements to, and
/// the values that measuring and placing read. A layout effect reaches nothing else of the desktop
/// system.
#[derive(Debug)]
pub struct LayoutPass<'a> {
    #[debug(skip)]
    layout_state: &'a mut DesktopLayoutState,
    aggregates: &'a mut Aggregates,
    desktop_presenter: &'a mut DesktopPresenter,
    reparenting: &'a mut Reparenting,
    inputs: LayoutInputs,
}

impl LayoutPass<'_> {
    /// Measures one layout target in a bottom-up pass and marks the effects that read its result.
    ///
    /// If any direct child is still unmeasured, this does not measure the target yet. Instead, it
    /// marks `Measure` for each missing child, whose measure marks this target's again.
    ///
    /// Once all children are measured, this measures `target`, always marks `Place(target)`, and
    /// marks `Measure(parent)` only when the measured size changed.
    pub fn measure(&mut self, target: DesktopTarget) {
        let missing_children = self
            .layout_state
            .missing_child_measures(&target, &self.aggregates.hierarchy);
        if !missing_children.is_empty() {
            for child in missing_children {
                DesktopEffect::Measure(child).mark();
            }
            return;
        }

        let algorithm = layout_algorithm(self.desktop_presenter, self.aggregates, self.inputs);

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
    pub fn place(&mut self, root: DesktopTarget) {
        let algorithm = layout_algorithm(self.desktop_presenter, self.aggregates, self.inputs);

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
    pub fn reparent(&mut self, target: DesktopTarget) {
        self.reparenting
            .take_rebase(&target)
            .expect("a reparent effect follows a re-parent")
            .apply(self.aggregates, self.layout_state);
        DesktopEffect::ApplyLayout(target).mark();
    }

    /// Applies one target's local placement to the renderer.
    ///
    pub fn apply(
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
}

/// The layout algorithm over the desktop presenter and the aggregates. It takes them one by one, so
/// the layout state stays free to be borrowed mutably beside it.
fn layout_algorithm<'a>(
    desktop: &'a DesktopPresenter,
    aggregates: &'a Aggregates,
    inputs: LayoutInputs,
) -> DesktopLayoutAlgorithm<'a> {
    DesktopLayoutAlgorithm {
        desktop,
        aggregates,
        context: aggregates.layout_context(inputs),
    }
}

impl Aggregates {
    pub(super) fn layout_context(&self, inputs: LayoutInputs) -> LayoutContext<'_> {
        let LayoutInputs {
            default_panel_size,
            window_size,
            focused_instance,
        } = inputs;
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
