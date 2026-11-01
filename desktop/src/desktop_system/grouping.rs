//! Grouping wraps slots into a new project (Cmd+G, ADR 0026). The new project takes the slots'
//! place, so they stay where they were on the desktop, one nesting level deeper.
//!
//! Ungrouping is the inverse (Cmd+Shift+G). It moves a project's slots into the project's parent,
//! starting at the project's own slot, removes the project, and selects the ungrouped slots.

use euclid::{point2, vec2};

use super::change::{Changes, ConfigurationChange, DesktopChange, TopologyChange, set_focus};
use super::configuration_planning::plan_add_project_topology;
use super::reparenting::Reparenting;
use super::selection::Selection;
use super::{DesktopTarget, FocusIntent, FocusableTarget, KeyboardFocusReason, ProjectPart};
use crate::policy::when::UngroupingFocus;
use crate::projects::{
    ProjectId, RuntimeConfiguration, SlotContent, SlotPosition, group_landings, matrix_bounding_box,
};

/// Plans grouping the `selected_slots` of `parent` into a new project named `name`, and focusing it
/// when `focus` follows.
///
/// The changes run in this order, because each one needs what the one before it creates. The
/// `Group` change creates the project and its presenter. The topology change adds the project's
/// nodes under the parent's matrix. The re-parents then move each slot's node and presenter under
/// the new project's matrix, which keeps its running instances and lets it slide into place.
pub fn plan(
    parent: ProjectId,
    selected_slots: Vec<SlotPosition>,
    project: ProjectId,
    name: String,
    focus: FocusIntent,
    configuration: &RuntimeConfiguration,
) -> Changes {
    let (_, landings) = group_landings(&selected_slots);

    let mut changes = Changes::Empty;
    changes <<= ConfigurationChange::Group {
        parent,
        slots: selected_slots,
        project,
        name,
    };
    changes += plan_add_project_topology(project, DesktopTarget::matrix(parent));
    for (outer, inner) in landings {
        changes += Reparenting::plan((parent, outer), (project, inner), configuration);
    }
    if focus == FocusIntent::Follow {
        changes += set_focus(
            Some(FocusableTarget::Project(project)),
            KeyboardFocusReason::AddSlotContent,
        );
    }
    changes
}

/// Plans ungrouping the non-empty `project`, and selecting its ungrouped slots when the focus moves
/// to them.
///
/// The changes run in this order, because each one needs the state the one before it leaves:
///
/// 1. When the focus moves, it moves to a slot at a corner of the ungrouped slots (see
///    [`ungrouped_selection`]). The project's removal would otherwise move the focus to the parent
///    project.
/// 2. Slots of the parent that are in the way are pushed aside.
/// 3. Each of the project's slots is re-parented under the parent's matrix. This happens while the
///    project is still placed, so the slots slide from where they are.
/// 4. The project's topology node is removed. Its slots are no longer below it.
/// 5. The `Ungroup` change moves the slots into the parent's configuration and removes the
///    project there.
/// 6. When the focus moved, the selection reaches from the focused slot to the opposite corner. It
///    comes last, so that it survives the focus and configuration changes before it.
pub fn plan_ungroup(
    project: ProjectId,
    focus: UngroupingFocus,
    configuration: &RuntimeConfiguration,
) -> Changes {
    let (parent, slot) = configuration
        .slot_of_content(project)
        .expect("an ungrouped project sits in a slot");
    let landings = configuration.ungroup_landings(project, slot);
    let landing_slots: Vec<_> = landings.iter().map(|(_, landing)| *landing).collect();

    let mut changes = Changes::Empty;
    let selection = match focus {
        UngroupingFocus::MoveToUngroupedSlots => {
            let (focused, selection) =
                ungrouped_focus(project, slot, &landings, &landing_slots, configuration);
            changes += set_focus(
                Some(focused.focusable_target()),
                KeyboardFocusReason::Ungroup,
            );
            selection
        }
        UngroupingFocus::Keep => None,
    };
    for (from, to) in configuration.ungroup_pushes(parent, slot, &landing_slots) {
        changes <<= ConfigurationChange::MoveSlot {
            source: (parent, from),
            dest: (parent, to),
        };
    }
    for (inner, landing) in landings {
        changes += Reparenting::plan((project, inner), (parent, landing), configuration);
    }
    changes <<= TopologyChange::Remove(DesktopTarget::Project(project, ProjectPart::Project));
    changes <<= ConfigurationChange::Ungroup { parent, slot };
    if let Some(ungrouped) = selection {
        changes <<= DesktopChange::SetSelection(ungrouped);
    }
    changes
}

/// The ungrouped slot content that takes the focus, and the selection that covers the ungrouped
/// slots from there. `project` sits at `slot`, and `landings` pairs each of its slots with where it
/// lands in the parent.
fn ungrouped_focus(
    project: ProjectId,
    slot: SlotPosition,
    landings: &[(SlotPosition, SlotPosition)],
    landing_slots: &[SlotPosition],
    configuration: &RuntimeConfiguration,
) -> (SlotContent, Option<Selection>) {
    let selection = ungrouped_selection(landing_slots);
    // Without an assigned corner, the slot that lands on the project's own slot takes the focus, or
    // else the first slot.
    let focused_landing = selection.map(|(corner, _)| corner).unwrap_or_else(|| {
        landing_slots
            .iter()
            .copied()
            .find(|landing| *landing == slot)
            .or(landing_slots.first().copied())
            .expect("only a project with slots is ungrouped")
    });
    let (focused_inner, _) = landings
        .iter()
        .find(|(_, landing)| *landing == focused_landing)
        .expect("the focused landing belongs to a slot");
    let focused = configuration
        .content_at(project, *focused_inner)
        .expect("a landing belongs to a slot");
    (focused, selection.map(|(_, ungrouped)| ungrouped))
}

/// The selection that covers all ungrouped slots at `landings` (ADR 0026): the landing at a corner
/// of their bounding box, which takes the focus, and the selection that reaches the opposite
/// corner. The corners are tried from the top-left, then the top-right, the bottom-left, and the
/// bottom-right. `None` when every corner is unassigned.
fn ungrouped_selection(landings: &[SlotPosition]) -> Option<(SlotPosition, Selection)> {
    let bounds = matrix_bounding_box(landings.iter().map(|&landing| landing.into()));
    let (first, last) = (bounds.min, bounds.max - vec2(1, 1));
    let top_right = point2(last.x, first.y);
    let bottom_left = point2(first.x, last.y);
    [
        (first, last),
        (top_right, bottom_left),
        (bottom_left, top_right),
        (last, first),
    ]
    .into_iter()
    .find(|(corner, _)| landings.contains(&(*corner).into()))
    .map(|(corner, opposite)| {
        let offset = opposite.to_i32() - corner.to_i32();
        (corner.into(), Selection::from_diagonal(offset))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(column: u32, row: u32) -> SlotPosition {
        SlotPosition { column, row }
    }

    /// The focus takes the first assigned corner in reading order, and the selection reaches the
    /// opposite corner. Without an assigned corner, nothing is selected.
    #[test]
    fn the_ungrouped_selection_starts_at_the_first_assigned_corner() {
        assert_eq!(
            ungrouped_selection(&[at(1, 1), at(2, 2)]),
            Some((at(1, 1), Selection::from_diagonal(vec2(1, 1))))
        );
        assert_eq!(
            ungrouped_selection(&[at(2, 1), at(1, 2)]),
            Some((at(2, 1), Selection::from_diagonal(vec2(-1, 1))))
        );
        assert_eq!(
            ungrouped_selection(&[at(1, 0), at(0, 1), at(2, 1), at(1, 2)]),
            None
        );
        assert_eq!(
            ungrouped_selection(&[at(3, 0)]),
            Some((at(3, 0), Selection::default()))
        );
    }
}
