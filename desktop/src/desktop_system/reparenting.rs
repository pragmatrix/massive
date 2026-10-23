//! Re-parenting moves slot content into the matrix of another project. The content keeps
//! everything below it, its running instances, and the keyboard focus. On screen, it slides from
//! its old place to its new one.
//!
//! A re-parent happens in three steps, at three different times, and this module provides each
//! of them:
//!
//! 1. Planning turns a move between two projects, a `MoveSlot` or a grouping, into a
//!    `TopologyChange::Reparent`.
//! 2. Applying that change moves the target in the hierarchy and attaches its presenter's scene
//!    location to the new matrix. It also reports the target as re-parented.
//! 3. The report becomes a `Reparent` effect, which runs after layout has placed the new parent.
//!    It converts the presenter's on-screen position into the new parent's coordinates, so that
//!    the presenter's next layout animates from where it is.

use std::collections::HashMap;

use anyhow::Result;
use massive_geometry::Transform;

use super::change::{Changes, DesktopChange, TopologyChange};
use super::change_surface::ChangeSurface;
use super::layout_state::DesktopLayoutState;
use super::{Aggregates, DesktopTarget, ProjectPart};
use crate::projects::{ProjectId, RuntimeConfiguration, SlotContent, SlotPosition};

/// The re-parented targets that still wait for step 3. For each target, it keeps the absolute
/// transform of the previous parent, captured before the move. The previous parent may be gone by
/// the time step 3 runs.
///
/// Known gap: when content is removed after its re-parent but before step 3, its entry stays
/// behind. This is harmless. Launcher and project ids are never reused, and a target that is
/// re-parented again overwrites its entry.
#[derive(Debug, Default)]
pub struct Reparenting {
    previous_parent_origins: HashMap<DesktopTarget, Transform>,
}

impl Reparenting {
    /// Step 1. Returns the topology change for slot content that moves from `source` to `dest`,
    /// either by a `MoveSlot` or by grouping. In the topology, the children of a matrix are its
    /// slots, so content that moves to another project needs a new topology parent. A move within
    /// one matrix needs no change.
    pub fn plan(
        (source_parent, source_slot): (ProjectId, SlotPosition),
        (dest_parent, _): (ProjectId, SlotPosition),
        configuration: &RuntimeConfiguration,
    ) -> Changes {
        if source_parent == dest_parent {
            return Changes::Empty;
        }
        let content = configuration
            .content_at(source_parent, source_slot)
            .expect("a movable slot has content");
        DesktopChange::Topology(TopologyChange::Reparent {
            what: content.target(),
            under: DesktopTarget::matrix(dest_parent),
        })
        .into()
    }

    /// Step 2. Moves `what` under `under` in the hierarchy and attaches its presenter's scene
    /// location to the new matrix. The presenter's layout transform is relative to its matrix, so
    /// the presenter has to hang under the matrix it now belongs to. Instances and nested slots
    /// hang under the presenter and follow it.
    ///
    /// The returned surface lists `what` as re-parented. The transaction turns that into the
    /// `Reparent` effect of step 3.
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
        let previous_origin =
            layout_state.absolute_origin_transform(&previous, &aggregates.hierarchy);

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

    /// Returns the pending step 3 for `target`, if it was re-parented. Each pending step is handed
    /// out only once.
    pub fn take_rebase(&mut self, target: &DesktopTarget) -> Option<Rebase> {
        self.previous_parent_origins
            .remove_entry(target)
            .map(|(target, previous_parent_origin)| Rebase {
                target,
                previous_parent_origin,
            })
    }
}

/// A pending step 3 for one target. It can only be obtained from [`Reparenting::take_rebase`].
/// This makes sure that the conversion runs exactly once, and only for re-parented content.
#[must_use]
#[derive(Debug)]
pub struct Rebase {
    target: DesktopTarget,
    previous_parent_origin: Transform,
}

impl Rebase {
    /// Converts the target's on-screen transform into the coordinates of its new parent. This runs
    /// right before the target's new layout is applied, so that the layout animates from where
    /// the target is on screen.
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
