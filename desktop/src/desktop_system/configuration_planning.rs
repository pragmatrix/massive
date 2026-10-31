//! The one planner of configuration commands (ADR 0024). It expands each [`ConfigurationCommand`]
//! into the primitive configuration changes of ADR 0011, together with the topology and focus
//! changes the command needs. The keyboard, the CLI, the tools, and the boot replay only name the
//! slots they mean and issue a command, so an operation ends the same way whichever of them issued
//! it. A command the desktop policy refuses plans [`Planned::Refused`] with the reason, so every
//! issuer learns why nothing changes.

use anyhow::{Result, bail};
use derive_more::{Display, From};

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
use crate::policy::constants;
use crate::policy::when::{
    self, NestingProjects, RemovingSlotContent, SlotSuccessor, UngroupingProject,
};
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

/// What planning a configuration command came to. Both [`Changes`] and a [`Refusal`] convert into
/// it, so a planning function returns either with `.into()`.
#[derive(Debug, From)]
pub enum Planned {
    Changes(Changes),
    /// Nothing changes, for the reason the refusal names.
    Refused(Refusal),
}

/// Why the planner refused a command. Its text is a sentence for an agent or the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum Refusal {
    #[display("The slot is unassigned.")]
    SlotUnassigned,
    #[display("The move names the same slot as source and destination.")]
    SameSlot,
    #[display(
        "The destination slot is assigned and lies in another project. Contents of two projects \
         are not exchanged."
    )]
    ExchangeAcrossProjects,
    #[display("A project cannot move into its own subtree.")]
    MoveIntoOwnSubtree,
    #[display("None of the slots is assigned.")]
    NothingToGroup,
    #[display("Instances run below the content. Stop them first.")]
    InstancesRunBelow,
    #[display(
        "Projects would nest more than {} levels below the root project.",
        constants::configuration::MAX_PROJECT_DEPTH
    )]
    NestingTooDeep,
    #[display("An MCP root cannot be ungrouped. Disable MCP for it first.")]
    UngroupingMcpRoot,
    #[display("The root project cannot be an MCP root.")]
    RootProjectAsMcpRoot,
    #[display("MCP roots cannot nest.")]
    NestedMcpRoot,
    #[display("The content or the run in front of it would move past the matrix's edge.")]
    PushPastEdge,
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
    /// How deep the deepest project lies after `command`, counted from the root project, for a
    /// command that can nest projects deeper. `None` for every other command.
    pub fn deepest_project_after(&self, command: &ConfigurationCommand) -> Option<usize> {
        let configuration = self.configuration;
        // Placed content that holds no project nests nothing deeper, even in a project that lies
        // past the limit already.
        let placed_under = |parent: ProjectId, height: usize| {
            (height > 0).then(|| configuration.project_depth(parent) + height)
        };
        // Assigned content is new, so a project brings one level and nothing below it.
        let assigned_height = |assignment: &SlotAssignment| match assignment {
            SlotAssignment::Project { .. } => 1,
            SlotAssignment::Launcher { .. } => 0,
        };
        match command {
            ConfigurationCommand::AssignSlot {
                parent: Some(parent),
                content,
                ..
            } => placed_under(*parent, assigned_height(content)),
            ConfigurationCommand::AddNextTo {
                anchor, content, ..
            } => {
                let (parent, _) = configuration
                    .slot_of_content(*anchor)
                    .expect("the anchor of an addition sits in a slot");
                placed_under(parent, assigned_height(content))
            }
            // The new project takes the grouped contents one level deeper.
            ConfigurationCommand::Group { parent, slots, .. } => placed_under(
                *parent,
                1 + slots
                    .iter()
                    .filter_map(|&slot| configuration.content_at(*parent, slot))
                    .map(|content| configuration.nesting_height(content))
                    .max()
                    .unwrap_or(0),
            ),
            ConfigurationCommand::MoveSlot { source, dest } if source.0 != dest.0 => {
                let content = configuration
                    .content_at(source.0, source.1)
                    .expect("a move starts at an assigned slot");
                placed_under(dest.0, configuration.nesting_height(content))
            }
            _ => None,
        }
    }

    /// Plans the changes that carry out `command`, including where the focus goes afterwards, or
    /// refuses it.
    pub fn plan(&self, command: ConfigurationCommand) -> Result<Planned> {
        if when::nesting_projects(self.deepest_project_after(&command))
            == NestingProjects::RefuseTooDeep
        {
            return Ok(Refusal::NestingTooDeep.into());
        }
        let planned = match command {
            ConfigurationCommand::AssignSlot {
                parent,
                slot,
                content,
                shift,
            } => match parent {
                Some(parent) => self.plan_assign_slot(parent, slot, content, shift)?,
                None => self.plan_root_creation(slot, content)?.into(),
            },
            ConfigurationCommand::ClearSlot {
                parent,
                slot,
                shift,
            } => self.plan_clear_slot(parent, slot, shift)?,
            ConfigurationCommand::MoveSlot { source, dest } => self.plan_move_slot(source, dest),
            ConfigurationCommand::MoveSlots {
                parent,
                block,
                direction,
            } => self.plan_move_slots(parent, block, direction).into(),
            ConfigurationCommand::PushSlot {
                parent,
                slot,
                direction,
            } => self.plan_push_slot(parent, slot, direction),
            ConfigurationCommand::AddNextTo {
                anchor,
                content,
                focus,
            } => self.plan_add_next_to(anchor, content, focus)?,
            ConfigurationCommand::Group {
                parent,
                slots,
                project,
                name,
                focus,
            } => self.plan_group(parent, slots, project, name, focus),
            ConfigurationCommand::Ungroup { project, focus } => {
                self.plan_ungroup(project, focus)?
            }
            ConfigurationCommand::SetStartupPath(path) => {
                let mut changes = Changes::Empty;
                changes <<= ConfigurationChange::SetStartupPath(path);
                changes.into()
            }
            ConfigurationCommand::ConfigureMcp { project, mcp } => {
                self.plan_configure_mcp(project, mcp)
            }
        };
        Ok(planned)
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

    /// A move onto an assigned slot of the same project exchanges the two contents, so an issuer
    /// only names where the content goes. Contents of two projects are never exchanged.
    fn plan_move_slot(
        &self,
        source: (ProjectId, SlotPosition),
        dest: (ProjectId, SlotPosition),
    ) -> Planned {
        let configuration = self.configuration;
        if source == dest {
            return Refusal::SameSlot.into();
        }
        if configuration.content_at(source.0, source.1).is_none() {
            return Refusal::SlotUnassigned.into();
        }
        let mut changes = Changes::Empty;
        match configuration.content_at(dest.0, dest.1) {
            Some(_) if source.0 != dest.0 => {
                return Refusal::ExchangeAcrossProjects.into();
            }
            Some(_) => {
                changes <<= ConfigurationChange::SwapSlots {
                    parent: source.0,
                    a: source.1,
                    b: dest.1,
                };
            }
            None if !configuration.can_move_slot(source, dest) => {
                return Refusal::MoveIntoOwnSubtree.into();
            }
            None => {
                changes += Reparenting::plan(source, dest, configuration);
                changes <<= ConfigurationChange::MoveSlot { source, dest };
            }
        }
        changes.into()
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

    /// A push of an unassigned slot, or one that would move content out of the matrix, left of its
    /// first column or above its first row, is refused.
    fn plan_push_slot(
        &self,
        parent: ProjectId,
        slot: SlotPosition,
        direction: Direction,
    ) -> Planned {
        if self.configuration.content_at(parent, slot).is_none() {
            return Refusal::SlotUnassigned.into();
        }
        match self.slot_shift_sequence(parent, slot, direction) {
            Ok(changes) => changes.into(),
            Err(_) => Refusal::PushPastEdge.into(),
        }
    }

    fn plan_add_next_to(
        &self,
        anchor: SlotContent,
        content: SlotAssignment,
        focus: FocusIntent,
    ) -> Result<Planned> {
        let (parent, slot) = self
            .configuration
            .slot_of_content(anchor)
            .expect("the anchor of an addition sits in a slot");

        let added = content.content().focusable_target();
        let next_to = SlotPosition {
            column: slot.column + 1,
            row: slot.row,
        };
        let mut changes =
            match self.plan_assign_slot(parent, next_to, content, SlotShift::default())? {
                Planned::Changes(changes) => changes,
                refused => return Ok(refused),
            };
        if focus == FocusIntent::Follow {
            changes += set_focus(Some(added), KeyboardFocusReason::AddSlotContent);
        }
        Ok(changes.into())
    }

    /// Only the assigned slots among `slots` are grouped, in the matrix's slot order, and a grouping
    /// without any is refused.
    fn plan_group(
        &self,
        parent: ProjectId,
        slots: Vec<SlotPosition>,
        project: ProjectId,
        name: String,
        focus: FocusIntent,
    ) -> Planned {
        let assigned: Vec<SlotPosition> = self
            .configuration
            .slots_ordered(parent)
            .map(|(slot, _)| slot)
            .filter(|slot| slots.contains(slot))
            .collect();
        if assigned.is_empty() {
            return Refusal::NothingToGroup.into();
        }
        grouping::plan(parent, assigned, project, name, focus, self.configuration).into()
    }

    fn plan_ungroup(&self, project: ProjectId, focus: FocusIntent) -> Result<Planned> {
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
                Ok(grouping::plan_ungroup(project, focus, configuration).into())
            }
            UngroupingProject::RefuseWhileMcpRoot => Ok(Refusal::UngroupingMcpRoot.into()),
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
    fn plan_configure_mcp(&self, project: ProjectId, mcp: Option<McpConfiguration>) -> Planned {
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
                MakingMcpRoot::RefuseRootProject => {
                    return Refusal::RootProjectAsMcpRoot.into();
                }
                MakingMcpRoot::RefuseNested => return Refusal::NestedMcpRoot.into(),
            }
        }
        let mut changes = Changes::Empty;
        changes <<= ConfigurationChange::ConfigureMcp { project, mcp };
        changes.into()
    }

    fn plan_assign_slot(
        &self,
        parent: ProjectId,
        slot: SlotPosition,
        content: SlotAssignment,
        shift: SlotShift,
    ) -> Result<Planned> {
        let mut changes = Changes::Empty;
        // Only assigned content that is live in the scene is displaced. At boot, the parsed
        // configuration already holds slots whose scene the replay is still building.
        let displaced = self
            .configuration
            .content_at(parent, slot)
            .filter(|content| self.hierarchy.exists(&content.target()));
        if displaced.is_some() {
            match shift {
                SlotShift::Shift => {
                    changes += self.slot_shift_sequence(parent, slot, Direction::Right)?;
                }
                SlotShift::Keep => match self.plan_clear_slot(parent, slot, SlotShift::Keep)? {
                    Planned::Changes(cleared) => changes += cleared,
                    refused => return Ok(refused),
                },
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
        Ok(changes.into())
    }

    /// Clearing an unassigned slot is refused, and so is removing content while instances run
    /// below it (see `when::removing_slot_content`).
    fn plan_clear_slot(
        &self,
        parent: ProjectId,
        slot: SlotPosition,
        shift: SlotShift,
    ) -> Result<Planned> {
        let Some(content) = self.configuration.content_at(parent, slot) else {
            return Ok(Refusal::SlotUnassigned.into());
        };
        let instances_run_below = self.hierarchy.has_instances_below(&content.target());
        if when::removing_slot_content(instances_run_below)
            == RemovingSlotContent::KeepWhileInstancesRun
        {
            return Ok(Refusal::InstancesRunBelow.into());
        }

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

        Ok(changes.into())
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
