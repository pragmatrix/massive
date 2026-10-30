//! The one planner of configuration commands (ADR 0024). It expands each [`ConfigurationCommand`]
//! into the primitive configuration changes of ADR 0011, together with the topology and focus
//! changes the command needs. The keyboard, the CLI, and the boot replay only name the slots they
//! mean and issue a command, so an operation ends the same way whichever of them issued it.

use anyhow::{Result, bail};
use log::warn;

use massive_applications::{Direction, McpConfiguration, SlotShift};

use super::change::{Changes, ConfigurationChange, DesktopChange, TopologyChange, set_focus};
use super::grouping;
use super::navigation::{FocusTargets, MatrixNavigation};
use super::reparenting::Reparenting;
use super::topology::DesktopTopology;
use super::{
    ConfigurationCommand, DesktopSystem, DesktopTarget, FocusIntent, FocusableTarget,
    KeyboardFocusReason, ProjectPart,
};
use crate::focus_path::PathResolver;
use crate::policy::when::{self, SlotSuccessor, UngroupingProject};
use crate::projects::{
    LaunchProfileId, MatrixRect, ProjectId, RuntimeConfiguration, SharedColumnCounts,
    SlotAssignment, SlotContent, SlotPosition, move_landings,
};
use crate::security::{self, MakingMcpRoot};

impl DesktopSystem {
    /// The configuration planner over the desktop state it reads.
    pub fn configuration_planning(&self) -> ConfigurationPlanning<'_> {
        ConfigurationPlanning {
            configuration: &self.aggregates.configuration,
            shared_column_counts: &self.aggregates.shared_column_counts,
            hierarchy: &self.aggregates.hierarchy,
            focus_targets: self.focus_targets(),
            focused: self.keyboard_focus.focused(),
        }
    }
}

/// Borrows the only state configuration planning reads: the configuration with its shared column
/// counts, the topology, the keyboard focus, and the focus slots and instance anchors the project
/// and launcher presenters remember. The window, the layout, and the camera stay out of its reach.
#[derive(Debug)]
pub struct ConfigurationPlanning<'a> {
    configuration: &'a RuntimeConfiguration,
    shared_column_counts: &'a SharedColumnCounts,
    hierarchy: &'a DesktopTopology,
    /// Picks the target that takes over the focus from removed content.
    focus_targets: FocusTargets<'a>,
    focused: Option<FocusableTarget>,
}

impl ConfigurationPlanning<'_> {
    /// Plans the changes that carry out `command`, including where the focus goes afterwards.
    pub fn plan(&self, command: ConfigurationCommand) -> Result<Changes> {
        let mut changes = Changes::Empty;
        match command {
            ConfigurationCommand::AssignSlot {
                parent,
                slot,
                content,
                shift,
            } => {
                let Some(parent) = parent else {
                    return self.plan_root_creation(slot, content);
                };
                changes += self.plan_assign_slot(parent, slot, content, shift)?;
            }
            ConfigurationCommand::ClearSlot {
                parent,
                slot,
                shift,
            } => {
                changes += self.plan_clear_slot(parent, slot, shift)?;
            }
            ConfigurationCommand::MoveSlot { source, dest } => {
                changes += self.plan_move_slot(source, dest);
            }
            ConfigurationCommand::SwapSlots { parent, a, b } => {
                changes <<= ConfigurationChange::SwapSlots { parent, a, b };
            }
            ConfigurationCommand::MoveSlots {
                parent,
                block,
                direction,
            } => {
                changes += self.plan_move_slots(parent, block, direction);
            }
            ConfigurationCommand::PushSlot {
                parent,
                slot,
                direction,
            } => {
                changes += self.plan_push_slot(parent, slot, direction);
            }
            ConfigurationCommand::AddNextTo {
                anchor,
                content,
                focus,
            } => {
                changes += self.plan_add_next_to(anchor, content, focus)?;
            }
            ConfigurationCommand::Group {
                parent,
                slots,
                name,
                focus,
            } => {
                changes += grouping::plan(parent, slots, name, focus, self.configuration);
            }
            ConfigurationCommand::Ungroup { project, focus } => {
                changes += self.plan_ungroup(project, focus)?;
            }
            ConfigurationCommand::SetStartupPath(path) => {
                changes <<= ConfigurationChange::SetStartupPath(path)
            }
            ConfigurationCommand::ConfigureMcp { project, mcp } => {
                changes += self.plan_configure_mcp(project, mcp);
            }
        }

        Ok(changes)
    }

    /// Creating the root is a parentless assignment of [`ProjectId::ROOT`] under
    /// the `Desktop` target: the root is hosted by no project and lives in no
    /// slot, so `slot` is carried through but never slotted.
    fn plan_root_creation(&self, slot: SlotPosition, content: SlotAssignment) -> Result<Changes> {
        let (id, name) = match content {
            SlotAssignment::Project { id, name } => (id, name),
            // Only boot plans a parentless assignment, and it only creates the root.
            SlotAssignment::Launcher { .. } => {
                panic!("a launcher is always assigned into a project's slot")
            }
        };
        if id != ProjectId::ROOT
            || self.hierarchy.exists(&DesktopTarget::Project(
                ProjectId::ROOT,
                ProjectPart::Project,
            ))
        {
            bail!(
                "Internal error (plan AssignSlot): the root project is created exactly once, as `AssignSlot {{ parent: None }}` with id {:?}",
                ProjectId::ROOT
            );
        }
        let mut changes: Changes = Changes::Empty;
        changes <<= ConfigurationChange::AssignSlot {
            parent: None,
            slot,
            assignment: SlotAssignment::Project { id, name },
        };
        changes += plan_add_project_topology(id, DesktopTarget::Desktop);
        Ok(changes)
    }

    /// Every issuer picks a valid move, so an invalid one is a bug of the issuer. A move onto an
    /// assigned slot is a `SwapSlots` instead.
    fn plan_move_slot(
        &self,
        source: (ProjectId, SlotPosition),
        dest: (ProjectId, SlotPosition),
    ) -> Changes {
        let configuration = self.configuration;
        assert!(
            configuration.content_at(dest.0, dest.1).is_none(),
            "a move lands on an unassigned slot, a swap exchanges assigned ones"
        );
        assert!(
            configuration.can_move_slot(source, dest),
            "a move starts at an assigned slot and lands in a live project outside its own subtree"
        );
        let mut changes = Reparenting::plan(source, dest, configuration);
        changes <<= ConfigurationChange::MoveSlot { source, dest };
        changes
    }

    /// A move that would take the block out of the columns and rows the matrix lays out, or that
    /// relocates no assigned slot, plans nothing. The moved contents keep their topology nodes,
    /// because they stay in the same matrix.
    fn plan_move_slots(
        &self,
        parent: ProjectId,
        block: MatrixRect,
        direction: Direction,
    ) -> Changes {
        let project = self
            .configuration
            .project(parent)
            .expect("the project of a moved block is configured");
        let slots = project.slot_contents().map(|(slot, _)| slot);
        let bounds = self.shared_column_counts.shared_matrix_bounds(project);
        let mut changes = Changes::Empty;
        if let Some(landings) = move_landings(block, direction, slots, bounds)
            && !landings.is_empty()
        {
            changes <<= ConfigurationChange::RelocateSlots { parent, landings };
        }
        changes
    }

    /// A push that would move content out of the matrix, left of its first column or above its
    /// first row, plans nothing.
    fn plan_push_slot(
        &self,
        parent: ProjectId,
        slot: SlotPosition,
        direction: Direction,
    ) -> Changes {
        match self.slot_shift_sequence(parent, slot, direction) {
            Ok(changes) => changes,
            Err(_) => {
                warn!(
                    "Ignoring {direction:?} push from slot ({}, {})",
                    slot.column, slot.row,
                );
                Changes::Empty
            }
        }
    }

    fn plan_add_next_to(
        &self,
        anchor: SlotContent,
        content: SlotAssignment,
        focus: FocusIntent,
    ) -> Result<Changes> {
        let (parent, slot) = self
            .configuration
            .slot_of_content(anchor)
            .expect("the anchor of an addition sits in a slot");

        let added = content.content().focusable_target();
        let mut changes = self.plan_assign_slot(
            parent,
            SlotPosition {
                column: slot.column + 1,
                row: slot.row,
            },
            content,
            SlotShift::default(),
        )?;
        if focus == FocusIntent::Follow {
            changes += set_focus(Some(added), KeyboardFocusReason::AddSlotContent);
        }
        Ok(changes)
    }

    fn plan_ungroup(&self, project: ProjectId, focus: FocusIntent) -> Result<Changes> {
        assert_ne!(
            project,
            ProjectId::ROOT,
            "the root project has no parent matrix to take its slots"
        );
        let configuration = self.configuration;
        match when::ungrouping_project(&configuration[project]) {
            UngroupingProject::MoveSlotsOut => {
                let focus_on_project = self.focused == Some(FocusableTarget::Project(project));
                let focus = when::ungrouping_focus(focus, focus_on_project);
                Ok(grouping::plan_ungroup(project, focus, configuration))
            }
            refusal @ UngroupingProject::RefuseWhileMcpRoot => {
                warn!(
                    "Refused to ungroup '{}': {refusal:?}, disable MCP for it first",
                    configuration[project].name
                );
                Ok(Changes::Empty)
            }
            UngroupingProject::RemoveProject => {
                let (parent, slot) = configuration
                    .slot_of_content(project)
                    .expect("a nested project sits in a slot");
                self.plan_clear_slot(parent, slot, SlotShift::Keep)
            }
        }
    }

    /// Disabling an MCP root is always allowed. Enabling one, or changing its access scopes, asks
    /// the `security` module whether the project may be an MCP root.
    fn plan_configure_mcp(&self, project: ProjectId, mcp: Option<McpConfiguration>) -> Changes {
        let configuration = self.configuration;
        if mcp.is_some() {
            let mcp_root_above = configuration
                .slot_of_content(project)
                .and_then(|(parent, _)| configuration.mcp_root_of(parent))
                .is_some();
            match security::making_mcp_root(
                project == ProjectId::ROOT,
                mcp_root_above,
                configuration.has_mcp_root_below(project),
            ) {
                MakingMcpRoot::Allow => {}
                refusal => {
                    warn!(
                        "Refused to make '{}' an MCP root: {refusal:?}",
                        configuration[project].name
                    );
                    return Changes::Empty;
                }
            }
        }
        let mut changes = Changes::Empty;
        changes <<= ConfigurationChange::ConfigureMcp { project, mcp };
        changes
    }

    fn plan_assign_slot(
        &self,
        parent: ProjectId,
        slot: SlotPosition,
        content: SlotAssignment,
        shift: SlotShift,
    ) -> Result<Changes> {
        let mut changes = Changes::Empty;
        // The parsed aggregate already has boot slots, while the topology
        // starts empty; only displace assigned content that is live in the scene.
        let assigned = self
            .configuration
            .content_at(parent, slot)
            .filter(|previous| self.hierarchy.exists(&previous.target()));
        if assigned.is_some() {
            match shift {
                SlotShift::Shift => {
                    changes += self.slot_shift_sequence(parent, slot, Direction::Right)?;
                }
                SlotShift::Keep => {
                    changes += self.plan_clear_slot(parent, slot, SlotShift::Keep)?;
                }
            }
        }

        changes <<= ConfigurationChange::AssignSlot {
            parent: Some(parent),
            slot,
            assignment: content.clone(),
        };
        let under = DesktopTarget::matrix(parent);
        match content.content() {
            SlotContent::Project(project) => changes += plan_add_project_topology(project, under),
            SlotContent::Launcher(launcher) => {
                changes <<= TopologyChange::Add {
                    what: DesktopTarget::Launcher(launcher),
                    under,
                    after: None,
                }
            }
        }
        Ok(changes)
    }

    fn plan_clear_slot(
        &self,
        parent: ProjectId,
        slot: SlotPosition,
        shift: SlotShift,
    ) -> Result<Changes> {
        let Some(content) = self.configuration.content_at(parent, slot) else {
            return Ok(Changes::Empty);
        };

        let mut changes = Changes::Empty;
        // Retarget focus before the content leaves the topology, so the removal does
        // not fall back to the generic parent retarget.
        changes += self.clear_slot_focus(content);
        match content {
            SlotContent::Launcher(launcher) => {
                changes += self.plan_remove_launcher_instances(launcher);
                changes <<= TopologyChange::Remove(DesktopTarget::Launcher(launcher));
            }
            SlotContent::Project(project) => {
                changes += self.plan_remove_project_content(project);
                changes <<= TopologyChange::Remove(project.into());
            }
        }
        changes <<= ConfigurationChange::ClearSlot { parent, slot };

        if shift == SlotShift::Shift {
            for (source, dest) in self.configuration.shifted_left_slots(parent, slot) {
                changes <<= ConfigurationChange::MoveSlot {
                    source: (parent, source),
                    dest: (parent, dest),
                };
            }
        }

        Ok(changes)
    }

    /// Retargets keyboard focus when the cleared slot holds it, so the removal does
    /// not leave the router pointing at a target about to leave the topology: the
    /// removed content gets a neighbouring replacement instead of its parent.
    fn clear_slot_focus(&self, content: SlotContent) -> Changes {
        let focused = self.focused.map(DesktopTarget::from);
        if !self
            .hierarchy
            .resolve_path(focused.as_ref())
            .contains(&content.target())
        {
            return Changes::Empty;
        }
        let replacement = self.slot_removal_focus(content);
        set_focus(Some(replacement), KeyboardFocusReason::InputTransition)
    }

    /// The focus replacement when slot `content` is removed, as the desktop policy
    /// chooses it from the nearest assigned slot in its matrix and its hosting project.
    fn slot_removal_focus(&self, content: SlotContent) -> FocusableTarget {
        let (parent, slot) = self
            .configuration
            .slot_of_content(content)
            .expect("removed content is assigned to a project slot");
        let matrix_navigation = MatrixNavigation::new(self.hierarchy, self.configuration);
        when::clearing_focused_slot()
            .into_iter()
            .find_map(|successor| match successor {
                SlotSuccessor::Neighbor(direction) => matrix_navigation
                    .navigate_within_matrix(parent, slot, direction, None)
                    .map(|replacement| self.focus_targets.slot_focus_target(replacement)),
                SlotSuccessor::HostingProject => Some(FocusableTarget::Project(parent)),
            })
            .expect("the hosting project always takes the focus")
    }

    fn plan_remove_launcher_instances(&self, launcher: LaunchProfileId) -> Changes {
        let mut changes = Changes::Empty;
        for instance in self.hierarchy.launcher_instances(launcher) {
            changes += [
                DesktopChange::Topology(TopologyChange::Remove(instance.into())),
                DesktopChange::HideInstance { launcher, instance },
                DesktopChange::ShutdownInstance(instance),
            ];
        }
        changes
    }

    fn plan_remove_project_content(&self, project: ProjectId) -> Changes {
        let mut changes = Changes::Empty;
        for slot in self.configuration.slots_ordered(project) {
            match slot.1 {
                SlotContent::Launcher(launcher) => {
                    changes += self.plan_remove_launcher_instances(launcher);
                    changes <<= TopologyChange::Remove(DesktopTarget::Launcher(launcher));
                }
                SlotContent::Project(nested) => {
                    changes += self.plan_remove_project_content(nested);
                    changes <<= TopologyChange::Remove(DesktopTarget::Project(
                        nested,
                        ProjectPart::Project,
                    ));
                }
            }
        }
        changes
    }

    fn slot_shift_sequence(
        &self,
        project: ProjectId,
        slot: SlotPosition,
        direction: Direction,
    ) -> Result<Changes> {
        let mut changes = Changes::Empty;
        for (source, dest) in self.configuration.shifted_slots(project, slot, direction)? {
            changes <<= ConfigurationChange::MoveSlot {
                source: (project, source),
                dest: (project, dest),
            };
        }
        Ok(changes)
    }
}

/// Adds a project and its header and matrix under the hosting target.
pub fn plan_add_project_topology(project: ProjectId, under: DesktopTarget) -> Changes {
    let mut changes = Changes::Empty;
    changes <<= TopologyChange::Add {
        what: project.into(),
        under,
        after: None,
    };
    changes <<= TopologyChange::AddNested {
        what: [
            DesktopTarget::header(project),
            DesktopTarget::matrix(project),
        ]
        .into(),
        under: project.into(),
    };
    changes
}
