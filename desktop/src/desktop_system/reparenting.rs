//! Re-parenting: slot content that moves into another project's matrix keeps its subtree, its
//! running instances and the keyboard focus, and slides from where it is drawn.
//!
//! The move touches three layers at different times, all routed through here: planning emits the
//! topology change, applying it re-parents the hierarchy and the presenter's scene location and
//! reports the target as re-parented, and the resulting `Reparent` effect re-bases the
//! presenter's animation into the new parent's space after every placement, once that parent is
//! placed.

use std::collections::HashMap;

use anyhow::Result;
use massive_geometry::Transform;

use super::change::{Changes, DesktopChange, TopologyChange};
use super::change_surface::ChangeSurface;
use super::layout_state::DesktopLayoutState;
use super::{Aggregates, DesktopTarget, ProjectPart};
use crate::projects::{MatrixPlacement, ProjectId, RuntimeConfiguration, SlotContent};

/// The re-parented targets whose animation still waits for its re-base, with their previous
/// parent's absolute origin transform.
///
/// Known gap: content removed between its re-parent and its next layout pass leaves its entry
/// behind. That is harmless, because launcher and project ids are never reused, and a target that
/// is re-parented again overwrites its entry.
#[derive(Debug, Default)]
pub struct Reparenting {
    previous_parent_origins: HashMap<DesktopTarget, Transform>,
}

impl Reparenting {
    /// The topology half of a `MoveSlot` the configuration already accepted: content that changes
    /// projects changes its topology parent, since a matrix's topology children are its slots.
    pub fn plan(
        (source_parent, source_placement): (ProjectId, MatrixPlacement),
        (dest_parent, _): (ProjectId, MatrixPlacement),
        configuration: &RuntimeConfiguration,
    ) -> Changes {
        if source_parent == dest_parent {
            return Changes::Empty;
        }
        let content = configuration
            .content_at(source_parent, source_placement)
            .expect("a movable slot has content");
        DesktopChange::Topology(TopologyChange::Reparent {
            what: content.target(),
            under: DesktopTarget::Project(dest_parent, ProjectPart::Matrix),
        })
        .into()
    }

    /// Re-parents `what` in the hierarchy and hangs its presenter under the new matrix, whose
    /// location its layout transform is relative to; instances and nested slots follow.
    ///
    /// The surface it returns carries `what` as re-parented, which becomes the effect that
    /// re-bases it once its new parent is placed.
    pub fn apply(
        &mut self,
        what: DesktopTarget,
        under: DesktopTarget,
        aggregates: &mut Aggregates,
        layout_state: &DesktopLayoutState,
    ) -> Result<ChangeSurface> {
        let DesktopTarget::Project(host, ProjectPart::Matrix) = under else {
            panic!("slot content is re-parented under a matrix: {under:?}");
        };
        let previous = aggregates
            .hierarchy
            .parent(&what)
            .cloned()
            .expect("a re-parented target has a parent");
        let previous_origin = layout_state.absolute_origin_transform(&previous, &aggregates.hierarchy);

        aggregates.hierarchy.reparent(&what, under.clone())?;

        let matrix_location = aggregates.project_matrix_location(host);
        match slot_content(&what) {
            SlotContent::Launcher(launcher) => {
                aggregates.launchers[&launcher].set_parent_location(&matrix_location)
            }
            SlotContent::Project(project) => {
                aggregates.projects[&project].set_parent_location(&matrix_location)
            }
        }
        self.previous_parent_origins
            .insert(what.clone(), previous_origin);

        let mut surface = ChangeSurface::measure(under);
        surface.reparented += what;
        Ok(surface)
    }

    /// The re-base `target` still owes since it was re-parented, handed out once.
    pub fn take_rebase(&mut self, target: &DesktopTarget) -> Option<Rebase> {
        self.previous_parent_origins
            .remove_entry(target)
            .map(|(target, previous_parent_origin)| Rebase {
                target,
                previous_parent_origin,
            })
    }
}

/// A pending re-base, only obtainable from [`Reparenting::take_rebase`], so a re-base happens
/// exactly once and only for re-parented content.
#[must_use]
#[derive(Debug)]
pub struct Rebase {
    target: DesktopTarget,
    previous_parent_origin: Transform,
}

impl Rebase {
    /// Re-expresses the target's drawn transform in its new parent's space right before its new
    /// layout is applied, so the layout animates from where it is drawn.
    pub fn apply(self, aggregates: &Aggregates, layout_state: &DesktopLayoutState) {
        let parent = aggregates
            .hierarchy
            .parent(&self.target)
            .expect("a re-parented target has a parent");
        let parent_origin = layout_state.absolute_origin_transform(parent, &aggregates.hierarchy);
        let previous_to_current = parent_origin.inverse() * self.previous_parent_origin;
        match slot_content(&self.target) {
            SlotContent::Launcher(launcher) => {
                aggregates.launchers[&launcher].rebase(previous_to_current)
            }
            SlotContent::Project(project) => {
                aggregates.projects[&project].rebase(previous_to_current)
            }
        }
    }
}

fn slot_content(target: &DesktopTarget) -> SlotContent {
    match target {
        DesktopTarget::Launcher(launcher) => SlotContent::Launcher(*launcher),
        DesktopTarget::Project(project, ProjectPart::Project) => SlotContent::Project(*project),
        _ => panic!("only slot content is re-parented: {target:?}"),
    }
}
