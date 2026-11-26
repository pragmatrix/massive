//! Re-parenting moves slot content into the matrix of another project. The content keeps
//! everything below it, its running instances, and the keyboard focus. On screen, it animates from
//! its old place to its new one.
//!
//! A re-parent is planned into a `TopologyChange::Reparent`, and applying that change moves the
//! content and converts its on-screen position into the new matrix's coordinates.

use anyhow::Result;
use massive_geometry::{SizedTransform, Transform};

use super::change::{Changes, DesktopChange, TopologyChange};
use super::effects::DesktopEffect;
use super::layout_state::DesktopLayoutState;
use super::{Aggregates, DesktopTarget, ProjectPart};
use crate::projects::{ProjectId, RuntimeConfiguration, SlotContent, SlotPosition};

/// Returns the topology change for slot content that moves from `source` into the matrix of
/// `dest_parent`, either by a `MoveSlot` or by grouping. In the topology, the children of a matrix
/// are its slots, so content that moves to another project needs a new topology parent. A move
/// within one matrix needs no change.
pub fn plan(
    (source_parent, source_slot): (ProjectId, SlotPosition),
    dest_parent: ProjectId,
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

/// Moves `what` into the matrix `under`, in the hierarchy and in the scene, and keeps it where it
/// is on screen.
///
/// The presenter's placement transform is relative to its matrix, so the presenter hangs under the
/// matrix it now belongs to. Instances and nested slots hang under the presenter and follow it.
///
/// The presenter's shown placement is converted into the new matrix's coordinates, so that its next
/// placement animates from where it is on screen.
pub fn apply(
    what: DesktopTarget,
    under: DesktopTarget,
    aggregates: &mut Aggregates,
    layout_state: &DesktopLayoutState,
) -> Result<()> {
    let DesktopTarget::Project(parent, ProjectPart::Matrix) = under else {
        panic!("slot content is re-parented under a matrix: {under:?}");
    };
    let Some(&DesktopTarget::Project(previous, ProjectPart::Matrix)) =
        aggregates.hierarchy.parent(&what)
    else {
        panic!("slot content hangs under a matrix: {what:?}");
    };
    let previous_matrix = shown_matrix_transform(previous, aggregates, layout_state)
        .expect("content leaves a matrix that is shown or placed");
    let previous_to_current = match shown_matrix_transform(parent, aggregates, layout_state) {
        Some(current) => current.inverse() * previous_matrix,
        None => {
            // Only grouping moves content into a matrix that is neither shown nor placed. The new
            // project's matrix starts where the previous matrix is
            // (`ProjectPresenter::animate_in_from`).
            assert_eq!(
                aggregates.hierarchy.parent(&parent.into()),
                Some(&DesktopTarget::matrix(previous)),
                "an unplaced matrix receives content only from its project's parent matrix"
            );
            Transform::IDENTITY
        }
    };

    aggregates.hierarchy.reparent(&what, under.clone())?;

    let matrix_location = aggregates.project_matrix_location(parent);
    match slot_content(&what) {
        SlotContent::Launcher(launcher) => {
            let presenter = aggregates
                .launchers
                .get_mut(&launcher)
                .expect("a re-parented launcher has a presenter");
            presenter.reparent(matrix_location, previous_to_current);
        }
        SlotContent::Project(project) => {
            let presenter = aggregates
                .projects
                .get_mut(&project)
                .expect("a re-parented project has a presenter");
            presenter.reparent(matrix_location, previous_to_current);
        }
    }

    DesktopEffect::Measure(under).mark();
    // Rebasing moved the shown placement off its target, and placing applies only changed
    // placements.
    DesktopEffect::ApplyPlacement(what).mark();
    Ok(())
}

/// The placement `target` shows now, relative to its parent. A target whose first placement is not
/// shown yet reads as its placement, because its first placement jumps there. `None` for a target
/// that is neither shown nor placed, such as a project that grouping created in this transaction.
///
/// Slot content, a project, and a project's matrix show placements.
pub fn shown_placement(
    target: &DesktopTarget,
    aggregates: &Aggregates,
    layout_state: &DesktopLayoutState,
) -> Option<SizedTransform> {
    let presented = match *target {
        DesktopTarget::Launcher(launcher) => aggregates.launchers[&launcher].presented_placement(),
        DesktopTarget::Project(project, ProjectPart::Project) => {
            aggregates.projects[&project].presented_placement()
        }
        DesktopTarget::Project(project, ProjectPart::Matrix) => {
            aggregates.projects[&project].matrix.presented_placement()
        }
        _ => panic!("only slot content and matrices show an animated placement: {target:?}"),
    };
    presented.or_else(|| {
        layout_state
            .is_placed(target)
            .then(|| DesktopLayoutState::sized_transform(&layout_state.local_placement(target)))
    })
}

/// The transform from the coordinates of `project`'s matrix into the desktop's, as it is shown
/// now. A parent that still animates is not at its placement yet, so the shown placements are
/// composed. `None` when a project on the way is neither shown nor placed.
fn shown_matrix_transform(
    project: ProjectId,
    aggregates: &Aggregates,
    layout_state: &DesktopLayoutState,
) -> Option<Transform> {
    let parent = match aggregates.hierarchy.parent(&project.into()) {
        Some(&DesktopTarget::Project(parent, ProjectPart::Matrix)) => {
            shown_matrix_transform(parent, aggregates, layout_state)?
        }
        // The root project hangs under the desktop's location, which every matrix shares, so it
        // cancels out of every conversion between two matrices.
        Some(DesktopTarget::Desktop) => Transform::IDENTITY,
        other => panic!("a project hangs under a matrix or the desktop: {other:?}"),
    };
    let project_layout = shown_placement(&project.into(), aggregates, layout_state)?;
    let matrix_layout = shown_placement(&DesktopTarget::matrix(project), aggregates, layout_state)?;
    Some(parent * project_layout.pivoted_at_origin() * matrix_layout.pivoted_at_origin())
}

fn slot_content(target: &DesktopTarget) -> SlotContent {
    match target {
        DesktopTarget::Launcher(launcher) => SlotContent::Launcher(*launcher),
        DesktopTarget::Project(project, ProjectPart::Project) => SlotContent::Project(*project),
        _ => panic!("only slot content is re-parented: {target:?}"),
    }
}
