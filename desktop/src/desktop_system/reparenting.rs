//! Re-parenting moves slot content into the matrix of another project. The content keeps
//! everything below it, its running instances, and the keyboard focus. On screen, it animates from
//! its old place to its new one.
//!
//! A re-parent is planned into a `TopologyChange::Reparent`, and applying that change moves the
//! content and converts its on-screen position into the new matrix's coordinates.

use anyhow::Result;
use massive_geometry::Transform;

use super::change::{Changes, DesktopChange, TopologyChange};
use super::effects::DesktopEffect;
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
/// The presenter's layout transform is relative to its matrix, so the presenter hangs under the
/// matrix it now belongs to. Instances and nested slots hang under the presenter and follow it.
///
/// The presenter's shown layout is converted into the new matrix's coordinates, so that its next
/// layout animates from where it is on screen.
pub fn apply(what: DesktopTarget, under: DesktopTarget, aggregates: &mut Aggregates) -> Result<()> {
    let DesktopTarget::Project(parent, ProjectPart::Matrix) = under else {
        panic!("slot content is re-parented under a matrix: {under:?}");
    };
    let Some(&DesktopTarget::Project(previous, ProjectPart::Matrix)) =
        aggregates.hierarchy.parent(&what)
    else {
        panic!("slot content hangs under a matrix: {what:?}");
    };
    let previous_to_current = presented_matrix_transform(parent, aggregates).inverse()
        * presented_matrix_transform(previous, aggregates);

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
    // Rebasing moved the shown layout off its target, and placing applies only changed placements.
    DesktopEffect::ApplyLayout(what).mark();
    Ok(())
}

/// The transform from the coordinates of `project`'s matrix into the desktop's, as it is presented
/// now. A parent that still animates is not at its placement yet, so the presented transforms are
/// composed. A matrix that was never presented, such as one grouping just created, presents its
/// parent's coordinates (`ProjectPresenter::animate_in_from`).
fn presented_matrix_transform(project: ProjectId, aggregates: &Aggregates) -> Transform {
    let presenter = &aggregates.projects[&project];
    let parent = match aggregates.hierarchy.parent(&project.into()) {
        Some(&DesktopTarget::Project(parent, ProjectPart::Matrix)) => {
            presented_matrix_transform(parent, aggregates)
        }
        // The root project hangs under the desktop's location, which every matrix shares, so it
        // cancels out of every conversion between two matrices.
        _ => Transform::IDENTITY,
    };
    parent * presenter.presented_transform() * presenter.matrix.presented_transform()
}

fn slot_content(target: &DesktopTarget) -> SlotContent {
    match target {
        DesktopTarget::Launcher(launcher) => SlotContent::Launcher(*launcher),
        DesktopTarget::Project(project, ProjectPart::Project) => SlotContent::Project(*project),
        _ => panic!("only slot content is re-parented: {target:?}"),
    }
}
