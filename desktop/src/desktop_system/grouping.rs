//! Grouping wraps slot content into a new project (Cmd+G). The new project takes the content's
//! placement, so the content stays where it was on the desktop, one nesting level deeper.
//!
//! Ungrouping is the inverse (Cmd+Shift+G). It unpacks a project's slots into the project's
//! parent at the project's placement and removes the project.

use super::change::{Changes, ConfigurationChange, TopologyChange, set_focus};
use super::command_dispatch::project_topology;
use super::reparenting::Reparenting;
use super::{DesktopTarget, FocusableTarget, KeyboardFocusReason, ProjectPart};
use crate::projects::{
    DEFAULT_NEW_PROJECT_NAME, GROUPED_PLACEMENT, ProjectId, RuntimeConfiguration, SlotContent,
};

/// Plans grouping `content` and focusing the new project.
///
/// The changes run in this order, because each one needs what the one before it creates. The
/// `Group` change creates the project and its presenter. The topology change adds the project's
/// nodes under the parent's matrix. The re-parent then moves the content's node and presenter
/// under the new project's matrix, which keeps its running instances and lets it slide into place.
pub fn plan(content: SlotContent, configuration: &RuntimeConfiguration) -> Changes {
    let (parent, placement) = configuration
        .slot_of_content(content)
        .expect("grouped content sits in a slot");
    let project = ProjectId::new();

    let mut changes = Changes::Empty;
    changes <<= ConfigurationChange::Group {
        parent,
        placement,
        project,
        name: DEFAULT_NEW_PROJECT_NAME.into(),
    };
    changes += project_topology(project, DesktopTarget::Project(parent, ProjectPart::Matrix));
    changes += Reparenting::plan(
        (parent, placement),
        (project, GROUPED_PLACEMENT),
        configuration,
    );
    changes += set_focus(
        Some(FocusableTarget::Project(project)),
        KeyboardFocusReason::AddSlotContent,
    );
    changes
}

/// Plans ungrouping the non-empty `project` and focusing one of its unpacked slots.
///
/// The changes run in this order, because each one needs the state the one before it leaves:
///
/// 1. The focus moves to the slot that lands on the project's placement, or else to the first
///    slot. The project's removal would otherwise move the focus to the parent project.
/// 2. Slots of the parent that are in the way are pushed aside.
/// 3. Each of the project's slots is re-parented under the parent's matrix. This happens while the
///    project is still placed, so the slots slide from where they are.
/// 4. The project's topology node is removed. Its slots are no longer below it.
/// 5. The `Ungroup` change moves the slots into the parent's configuration and removes the
///    project there.
pub fn plan_ungroup(project: ProjectId, configuration: &RuntimeConfiguration) -> Changes {
    let (parent, placement) = configuration
        .slot_of_content(project)
        .expect("an ungrouped project sits in a slot");
    let landings = configuration.ungroup_landings(project, placement);
    let (focused_inner, _) = landings
        .iter()
        .find(|(_, landing)| *landing == placement)
        .or(landings.first())
        .expect("only a project with slots is ungrouped");
    let focused = configuration
        .content_at(project, *focused_inner)
        .expect("a landing belongs to a slot");

    let mut changes = set_focus(
        Some(focused.focusable_target()),
        KeyboardFocusReason::Ungroup,
    );
    let landing_placements: Vec<_> = landings.iter().map(|(_, landing)| *landing).collect();
    for (from, to) in configuration.ungroup_pushes(parent, placement, &landing_placements) {
        changes <<= ConfigurationChange::MoveSlot {
            source: (parent, from),
            dest: (parent, to),
        };
    }
    for (inner, landing) in landings {
        changes += Reparenting::plan((project, inner), (parent, landing), configuration);
    }
    changes <<= TopologyChange::Remove(DesktopTarget::Project(project, ProjectPart::Project));
    changes <<= ConfigurationChange::Ungroup { parent, placement };
    changes
}
