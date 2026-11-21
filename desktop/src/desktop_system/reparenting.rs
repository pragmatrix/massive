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

/// Moves `what` under the matrix `under` in the hierarchy and attaches its presenter's scene
/// location to that matrix. The presenter's layout transform is relative to its matrix, so the
/// presenter has to hang under the matrix it now belongs to. Instances and nested slots hang under
/// the presenter and follow it.
///
/// The presenter's on-screen layout is converted into the new matrix's coordinates, so that its
/// next layout animates from where it is on screen.
pub fn apply(
    what: DesktopTarget,
    under: DesktopTarget,
    aggregates: &mut Aggregates,
    layout_state: &DesktopLayoutState,
) -> Result<()> {
    let DesktopTarget::Project(parent, ProjectPart::Matrix) = under else {
        panic!("slot content is re-parented under a matrix: {under:?}");
    };
    let previous = aggregates
        .hierarchy
        .parent(&what)
        .cloned()
        .expect("a re-parented target has a parent");
    let previous_to_current =
        previous_parent_to_matrix(&previous, parent, aggregates, layout_state);

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
    // The content's placement in the new matrix may equal its placement in the previous one, and
    // placing it would then not apply its layout again.
    DesktopEffect::ApplyLayout(what).mark();
    Ok(())
}

/// The transform from the coordinates of the `previous` parent into those of `parent`'s matrix, as
/// both are shown before the transaction's effects run.
fn previous_parent_to_matrix(
    previous: &DesktopTarget,
    parent: ProjectId,
    aggregates: &Aggregates,
    layout_state: &DesktopLayoutState,
) -> Transform {
    let matrix_target = DesktopTarget::matrix(parent);
    if !layout_state.is_placed(&matrix_target) {
        // Only grouping moves content into a matrix that is not placed yet. The new project's
        // matrix starts where `previous` is (`ProjectPresenter::animate_in_from`).
        assert_eq!(
            aggregates.hierarchy.parent(&parent.into()),
            Some(previous),
            "an unplaced matrix receives content only from its project's parent matrix"
        );
        return Transform::IDENTITY;
    }
    let hierarchy = &aggregates.hierarchy;
    let current_origin = layout_state.absolute_origin_transform(&matrix_target, hierarchy);
    let previous_origin = layout_state.absolute_origin_transform(previous, hierarchy);
    current_origin.inverse() * previous_origin
}

fn slot_content(target: &DesktopTarget) -> SlotContent {
    match target {
        DesktopTarget::Launcher(launcher) => SlotContent::Launcher(*launcher),
        DesktopTarget::Project(project, ProjectPart::Project) => SlotContent::Project(*project),
        _ => panic!("only slot content is re-parented: {target:?}"),
    }
}
