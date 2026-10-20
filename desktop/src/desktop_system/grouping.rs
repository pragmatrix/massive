//! Grouping wraps slot content into a new project (Cmd+G). The new project takes the content's
//! placement, so the content stays where it was on the desktop, one nesting level deeper.

use super::change::{Changes, ConfigurationChange, set_focus};
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
