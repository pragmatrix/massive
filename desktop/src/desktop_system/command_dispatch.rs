use std::fmt;

use anyhow::{Context, Result};
use log::{debug, warn};
use serde_json::json;

use super::change::set_focus;
use super::change::{
    Changes, ConfigurationChange, DesktopChange, InstancePresentation, ToggleFullScreenModeTarget,
    TopologyChange,
};
use super::selection::Selection;
use super::{
    ChangeSurface, ConfigurationCommand, DesktopCommand, DesktopSystem, DesktopTarget, FocusIntent,
    FocusableTarget, KeyboardFocusReason, Planned, SelectionOperation, ZoomLevel, view_size,
};
use crate::desktop_system::change_surface::{ConfigurationImpact, TargetSet};
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::instance_presenter::{InstanceKind, InstanceRoot};
use crate::policy::when::{self, TogglingFullScreen, Ungrouping};
use crate::projects::{
    DEFAULT_NEW_LAUNCHER_NAME, DEFAULT_NEW_PROJECT_NAME, LaunchProfile, LaunchProfileId,
    LauncherMode, LauncherPresenter, ProjectId, ProjectPresenter, SlotAssignment, SlotContent,
    SlotPosition,
};
use crate::security::{self, ConfiguringMcp};

use massive_applications::prelude::*;
use massive_applications::{
    ConfigurationRequest, ConfigurationTarget, CreationMode, InstanceChange, InstanceId,
    InstanceSubmission, SlotShift, ViewChange, ViewEvent, ViewRole,
};

/// The outcome of applying a change: its effects and any follow-up changes.
#[derive(Debug, Default)]
pub struct ChangeOutput {
    /// Additional changes to schedule.
    pub changes: Changes,
    pub surface: ChangeSurface,
}

impl ChangeOutput {
    pub fn measure(&mut self, target: DesktopTarget) {
        self.surface.size_invalid += target;
    }

    pub fn focus_changed(
        &mut self,
        previous: Option<DesktopTarget>,
        current: Option<DesktopTarget>,
    ) {
        if previous == current {
            return;
        }

        if let Some(previous) = previous {
            self.surface.size_invalid += previous;
        }
        if let Some(current) = current {
            self.surface.size_invalid += current;
        }
    }

    fn measures(measures: impl Into<TargetSet>) -> Self {
        Self {
            surface: ChangeSurface {
                size_invalid: measures.into(),
                ..Default::default()
            },
            ..Self::default()
        }
    }

    pub fn changes(changes: Changes) -> Self {
        Self {
            changes,
            ..Self::default()
        }
    }

    pub fn combine(&mut self, other: Self) {
        self.changes += other.changes;
        self.surface.combine(other.surface);
    }
}

impl DesktopSystem {
    /// Plans a command that a tool issued (ADR 0036). A refused configuration command comes back
    /// as [`Planned::Refused`], so the tool answers the agent with the reason instead of a success.
    pub fn plan_issued(&self, command: DesktopCommand) -> Result<Planned> {
        match command {
            DesktopCommand::Configuration(command) => self.configuration_planning().plan(command),
            command => self.plan(command).map(Planned::Changes),
        }
    }

    /// Plan the execution of a command. A refused configuration command plans nothing, and the
    /// refusal is logged.
    pub fn plan(&self, command: DesktopCommand) -> Result<Changes> {
        match command {
            DesktopCommand::Configuration(configuration_command) => Ok(changes_of(
                self.configuration_planning().plan(configuration_command)?,
            )),
            DesktopCommand::ConfigureSelection(operation) => {
                let Some(command) = self.command_for_selection(operation) else {
                    return Ok(Changes::Empty);
                };
                Ok(changes_of(self.configuration_planning().plan(command)?))
            }
            DesktopCommand::StartInstance {
                launcher,
                instance,
                root,
                parameters,
                kind,
                focus,
            } => {
                // An instance an agent starts does not appear next to, or slide out of, the
                // instance the user works in, which may lie outside the agent's MCP root.
                let originator_instance = match focus {
                    FocusIntent::Follow => self.focused_path().instance(),
                    FocusIntent::Keep => None,
                };
                let originating_details = originator_instance
                    .map(|originator| self.get_origination_details(launcher, originator));
                let insertion_pos = originating_details
                    .as_ref()
                    .map(|d| d.insertion_pos)
                    .unwrap_or(0);
                let (root, spawn) = match root {
                    Some(root) => (root, false),
                    // Rare case: only a spawned instance needs new scene objects, so the ambient
                    // change queue is used here instead of being threaded through `plan`.
                    None => (InstanceRoot::new(), true),
                };

                let mut changes: Changes = if spawn {
                    // The spawned application's `size_px` seeds its canvas: a
                    // fullscreen primary instance presents at the window below its
                    // title bar, not the panel, so it must start at that size or
                    // its first frames render panel-sized and reflow on Resized
                    // (ADR 0014, ADR 0019).
                    let initial_size_px = view_size(
                        kind.initial_full_screen_mode(
                            self.aggregates.configuration[launcher].full_screen_mode,
                        ),
                        self.default_panel_size,
                        self.window_state.inner_size,
                        self.aggregates.instance_title_bar_metrics.height,
                    );
                    let mut spawn_parameters = parameters.clone();
                    spawn_parameters.insert(
                        "size_px".to_string(),
                        json!([initial_size_px.width, initial_size_px.height]),
                    );
                    vec![DesktopChange::SpawnInstance {
                        instance,
                        root: root.clone(),
                        parameters: spawn_parameters,
                    }]
                } else {
                    Vec::new()
                }
                .into();

                changes += [
                    DesktopChange::PresentInstance(InstancePresentation {
                        launcher,
                        initial_center_translation: originating_details
                            .and_then(|od| od.initial_center_translation),
                        instance,
                        root,
                        parameters,
                        kind,
                    }),
                    DesktopChange::Topology(TopologyChange::Insert {
                        what: instance.into(),
                        at_index: insertion_pos,
                        under: launcher.into(),
                    }),
                    // ADR 0019: The title bar exists with the instance, ahead of its view.
                    DesktopChange::Topology(TopologyChange::Add {
                        what: DesktopTarget::title_bar(instance),
                        under: instance.into(),
                        after: None,
                    }),
                ];
                if focus == FocusIntent::Follow {
                    changes <<= DesktopChange::SetZoomLevel(ZoomLevel::Focus);
                    changes += set_focus(
                        Some(FocusableTarget::Instance(instance)),
                        KeyboardFocusReason::PresentInstance,
                    );
                }

                Ok(changes)
            }
            DesktopCommand::StopInstance(instance) => {
                let launcher = self.aggregates.hierarchy.launcher_of_instance(instance);

                // Set up a replacement focus first.
                //
                // Detail: This causes an unfocus event sent to the instance's view which may
                // unexpected while tear down.
                let replacement_focus = self.focused_target().and_then(|focused| {
                    self.aggregates
                        .hierarchy
                        .resolve_replacement_focus_for_stopping_instance(&focused, instance)
                });

                let mut changes = Changes::Empty;
                if let Some(focus) = replacement_focus {
                    changes += set_focus(Some(focus), KeyboardFocusReason::StopInstanceReplacement);
                }
                changes += [
                    DesktopChange::Topology(TopologyChange::Remove(instance.into())),
                    DesktopChange::HideInstance { launcher, instance },
                    DesktopChange::ShutdownInstance(instance),
                ];

                Ok(changes)
            }
            DesktopCommand::Navigate(direction) => self.plan_navigate(direction),
            DesktopCommand::ExtendSelection(direction) => Ok(self.selection.plan_extend(
                direction,
                self.keyboard_focus.focused(),
                &self.selection_context(),
            )),
            DesktopCommand::SelectAll(project) => Ok(Selection::plan_select_all(
                project,
                self.keyboard_focus.focused(),
                &self.selection_context(),
            )),
            DesktopCommand::CollapseSelection => {
                Ok(DesktopChange::SetSelection(Selection::default()).into())
            }
            DesktopCommand::Escape => Ok(self.zoom_navigation().plan_escape()),
            DesktopCommand::Zoom(zoom) => Ok(self.zoom_navigation().plan_zoom(zoom)),
            DesktopCommand::ToggleFullScreen => {
                let fully_in_on_instance = self.focused_path().instance().is_some()
                    && self.zoom_navigation().is_fully_zoomed_in();
                match when::toggling_full_screen(
                    self.window_state.is_fullscreen,
                    fully_in_on_instance,
                ) {
                    TogglingFullScreen::Mode => self.plan_toggle_full_screen_mode(),
                    TogglingFullScreen::Window => Ok(DesktopChange::ToggleWindowFullScreen.into()),
                }
            }
        }
    }

    /// The configuration command that `operation` stands for on the current selection (ADR 0024).
    /// `None` when the selection holds nothing the operation applies to, or when a desktop policy
    /// guard refuses it.
    fn command_for_selection(&self, operation: SelectionOperation) -> Option<ConfigurationCommand> {
        match operation {
            SelectionOperation::AddLauncher => {
                let focused = self.focused_target()?;
                let Some(anchor) = self.aggregates.hierarchy.slot_content_of_target(&focused)
                else {
                    warn!("{focused:?} sits in no slot to add next to");
                    return None;
                };
                Some(ConfigurationCommand::AddNextTo {
                    anchor,
                    content: new_slot_assignment(ConfigurationTarget::Launcher, None),
                    focus: FocusIntent::Follow,
                })
            }
            SelectionOperation::Remove => self.removal_command_for_focus(),
            SelectionOperation::Group => {
                let Some((parent, slots)) = self.keyboard_focus.focused().and_then(|focused| {
                    self.selection
                        .selected_slots(focused, &self.selection_context())
                }) else {
                    warn!("The focus sits in no slot to group");
                    return None;
                };
                Some(ConfigurationCommand::Group {
                    parent,
                    slots,
                    project: ProjectId::new(),
                    name: DEFAULT_NEW_PROJECT_NAME.into(),
                    focus: FocusIntent::Follow,
                })
            }
            SelectionOperation::Ungroup => self.ungroup_command_for_selection(),
            SelectionOperation::Move(direction) => {
                let Some((parent, block)) = self.keyboard_focus.focused().and_then(|focused| {
                    self.selection
                        .selected_rect(focused, &self.selection_context())
                }) else {
                    warn!("The focus sits in no slot to move");
                    return None;
                };
                Some(ConfigurationCommand::MoveSlots {
                    parent,
                    block,
                    direction,
                })
            }
        }
    }

    /// The removal of the focused slot's content that Cmd+Shift+W issues. The planner refuses it
    /// while instances run below the content.
    fn removal_command_for_focus(&self) -> Option<ConfigurationCommand> {
        // The root project sits in no slot, so the focus on it, or on the desktop, removes nothing.
        let content = self
            .aggregates
            .hierarchy
            .slot_content_of_target(&self.focused_target()?)?;
        let (parent, slot) = self
            .aggregates
            .configuration
            .slot_of_content(content)
            .expect("slot content of a live target sits in a slot");
        Some(ConfigurationCommand::ClearSlot {
            parent,
            slot,
            shift: SlotShift::default(),
        })
    }

    /// The ungrouping Cmd+Shift+G issues: of the focused project, or of the one whose slots are all
    /// selected (ADR 0029). Nothing for the root, which has no parent matrix to take its slots.
    fn ungroup_command_for_selection(&self) -> Option<ConfigurationCommand> {
        let focused = self.keyboard_focus.focused();
        let focused_project = match focused {
            Some(FocusableTarget::Project(project)) => Some(project),
            _ => None,
        };
        // Selecting all slots of the root does not count.
        let selected_project = focused
            .and_then(|focused| {
                self.selection
                    .fully_selected_project(focused, &self.selection_context())
            })
            .filter(|&project| project != ProjectId::ROOT);
        let project = match when::ungrouping(
            focused_project.is_some(),
            selected_project.is_some(),
            self.selection.is_extended(),
        ) {
            Ungrouping::UngroupFocusedProject => {
                focused_project.expect("the keyboard focus is on a project")
            }
            Ungrouping::UngroupSelectedProject => {
                selected_project.expect("the selection covers a project")
            }
            refusal @ Ungrouping::RefuseUnlessFocusedOrFullySelected => {
                return refused(refusal);
            }
        };
        (project != ProjectId::ROOT).then_some(ConfigurationCommand::Ungroup {
            project,
            focus: FocusIntent::Follow,
        })
    }

    /// Plans `ToggleFullScreenMode` (ADR 0014): a primary instance resolves to its
    /// launcher's mode; an assistant instance resolves to its own. A launcher
    /// without instances is a no-op.
    fn plan_toggle_full_screen_mode(&self) -> Result<Changes> {
        let mut changes: Changes = Changes::Empty;

        match self.keyboard_focus.focused() {
            Some(FocusableTarget::Launcher(launcher))
                if self
                    .aggregates
                    .hierarchy
                    .launcher_instances(launcher)
                    .next()
                    .is_some() =>
            {
                changes <<= DesktopChange::ToggleFullScreenMode(
                    ToggleFullScreenModeTarget::Launcher(launcher),
                );
            }
            Some(FocusableTarget::Instance(instance) | FocusableTarget::View(instance, _)) => {
                // An assistant toggles its own temporary mode; a base
                // instance toggles its launcher's mode (ADR 0014).
                let toggle = if self
                    .aggregates
                    .instances
                    .get(&instance)
                    .is_some_and(|presenter| presenter.kind() == InstanceKind::Assistant)
                {
                    ToggleFullScreenModeTarget::AssistantInstance(instance)
                } else {
                    ToggleFullScreenModeTarget::Launcher(
                        self.aggregates.hierarchy.launcher_of_instance(instance),
                    )
                };
                changes <<= DesktopChange::ToggleFullScreenMode(toggle);
            }
            _ => {}
        }

        Ok(changes)
    }

    pub fn apply_change(
        &mut self,
        change: DesktopChange,
        instance_manager: &mut InstanceManager,
    ) -> Result<ChangeOutput> {
        match change {
            DesktopChange::SpawnInstance {
                instance,
                root,
                parameters,
            } => {
                // Probably pull the name of the application into SpawnInstance?
                let application = self
                    .env
                    .applications
                    .get_named(&self.env.primary_application)
                    .context("Internal error, application not registered")?;

                instance_manager.spawn(
                    instance,
                    application,
                    CreationMode::New(parameters),
                    root.view_parent(),
                )?;
            }
            DesktopChange::ShutdownInstance(instance) => {
                // This might fail if StopInstance gets triggered with an instance that ended in
                // itself (shouldn't the instance_manager keep it until we finally free it).
                if let Err(e) = instance_manager.request_shutdown(instance) {
                    warn!("Failed to shutdown instance, it may be gone already: {e}");
                };
            }
            DesktopChange::PresentInstance(presentation) => {
                self.present_instance(presentation)?;
            }
            DesktopChange::HideInstance { launcher, instance } => {
                self.hide_instance(launcher, instance)?;
            }
            DesktopChange::SetFocus { target } => {
                let previous_focus = self.focused_target();
                self.focus(target, instance_manager)?;
                let current_focus = self.focused_target();

                let mut output = ChangeOutput::default();
                output.focus_changed(previous_focus, current_focus);

                return Ok(output);
            }
            DesktopChange::SetNavigationAffinity(column_affinity) => {
                self.navigation_control
                    .commit_column_affinity(column_affinity);
            }
            DesktopChange::SetSelection(selection) => {
                self.selection.set(selection);
                let mut output = ChangeOutput::default();
                output.surface.selection_set = true;
                return Ok(output);
            }
            DesktopChange::SetInputFocus(input_focus) => {
                self.keyboard_focus.set_input_focus(input_focus);
            }
            DesktopChange::SetZoomLevel(level) => {
                if self.zoom_level != level {
                    self.zoom_level = level;

                    return Ok(ChangeOutput::measures(TargetSet::from_iter(
                        self.focused_target(),
                    )));
                }
            }
            DesktopChange::ToggleFullScreenMode(target) => {
                let changed_targets: Vec<DesktopTarget> = match target {
                    ToggleFullScreenModeTarget::Launcher(launcher) => {
                        let mode = self.aggregates.configuration[launcher].full_screen_mode;
                        self.aggregates
                            .configuration
                            .launcher_mut(launcher)
                            .expect(
                                "the hierarchy's launcher of a focused target is in the configuration",
                            )
                            .full_screen_mode = mode.toggled();

                        let mut targets = vec![DesktopTarget::Launcher(launcher)];
                        for instance in self.aggregates.hierarchy.launcher_instances(launcher) {
                            targets.extend(self.instance_view_targets(instance));
                        }
                        targets
                    }
                    ToggleFullScreenModeTarget::AssistantInstance(instance) => {
                        self.toggle_assistant_full_screen_mode(instance);

                        self.instance_view_targets(instance)
                    }
                };

                // The fullscreen scale lives on the views' placements AND their
                // measurements (panel size vs window size), so the toggle must
                // measure the views: the Instance measure treats already-measured
                // children as valid, and Place(Instance) computed from a stale
                // window-size view measurement dead-ends the visor layout until
                // the next focus or rotation re-measures.
                // The live mode toggle is a configuration change the caller
                // persists (ADR 0013; 0014).
                let mut output = ChangeOutput::measures(TargetSet::from_iter(changed_targets));
                output.surface.configuration_impact = ConfigurationImpact::Changed;
                return Ok(output);
            }
            DesktopChange::WindowResized(window_state) => {
                self.window_state = window_state;
                // In Full Screen Mode the view and the title bar are measured at window resolution
                // (ADR 0014, ADR 0019). The measure pass keeps the cached sizes of measured
                // children, so the view and the title bar are measured explicitly. Instances in the
                // regular mode measure at the panel size, so their measurements stay unchanged.
                let views: TargetSet = self
                    .aggregates
                    .instances
                    .keys()
                    .flat_map(|&instance| self.instance_view_targets(instance))
                    .collect();
                let mut output = ChangeOutput::measures(views);
                output.surface.window_size_changed = true;
                return Ok(output);
            }
            DesktopChange::ToggleWindowFullScreen => {
                let mut output = ChangeOutput::default();
                output.surface.window_fullscreen_changed = true;
                return Ok(output);
            }
            DesktopChange::ResizeAll(size_px) => {
                self.default_panel_size = size_px;
                for (instance, presenter) in self.aggregates.instances.iter_mut() {
                    let Some(view) = presenter.primary_view_id() else {
                        continue;
                    };
                    if let Err(error) = instance_manager
                        .send_view_event((*instance, view), ViewEvent::Resized(size_px))
                    {
                        warn!("Failed to resize terminal instance {instance:?}: {error}");
                    }
                }
                // Root measurement otherwise reuses descendant measurements made for the previous
                // panel extent, leaving project and matrix slots at their old sizes.
                self.layout_state.clear();
                return Ok(ChangeOutput::measures(DesktopTarget::Desktop));
            }
            DesktopChange::Topology(change) => {
                let previous_focus = self.focused_target();
                // Design: That's somewhat unexpected here, that `apply_topology_change` changes
                // focus. Can we make this more obvious? We should combine the `instance_manager`
                // side effects perhaps.
                let surface = self.apply_topology_change(change, instance_manager)?;
                let current_focus = self.focused_target();

                let mut output = ChangeOutput {
                    surface,
                    ..Default::default()
                };
                output.focus_changed(previous_focus, current_focus);

                return Ok(output);
            }
            DesktopChange::ForwardEvents(transitions) => {
                let commands = self.forward_event_transitions(transitions, instance_manager)?;
                let mut changes = Changes::default();
                for command in commands {
                    changes += self.plan(command)?;
                }
                return Ok(ChangeOutput::changes(changes));
            }
            DesktopChange::IntegrateInstanceSubmission(instance_id, instance_submission) => {
                return self.apply_instance_submission(instance_id, instance_submission);
            }
            DesktopChange::Project(project_change) => {
                // A change elsewhere leaves the selection's slots where they are, so an agent
                // working in another project does not take the user's selection away (ADR 0036).
                let under_selection = !project_change.keeps_selection()
                    && self.project_holding_selection().is_some_and(|holding| {
                        project_change.touched_projects().contains(&holding)
                    });
                let configuration_impact = if under_selection {
                    ConfigurationImpact::ChangedUnderSelection
                } else {
                    ConfigurationImpact::Changed
                };
                let mut output = self.apply_project_change(project_change)?;
                output.surface.configuration_impact = configuration_impact;
                return Ok(output);
            }
        }

        Ok(ChangeOutput::default())
    }

    pub fn apply_topology_change(
        &mut self,
        change: TopologyChange,
        instance_manager: &InstanceManager,
    ) -> Result<ChangeSurface> {
        match change {
            TopologyChange::Add { what, under, after } => {
                if let Some(after) = after {
                    // Design: `under` can be resolved via `after`!
                    self.aggregates.hierarchy.add_after(after, what)?;
                } else {
                    self.aggregates.hierarchy.add(under.clone(), what)?;
                }
                Ok(ChangeSurface::measure(under))
            }
            TopologyChange::AddNested { what, under } => {
                self.aggregates.hierarchy.add_nested(under.clone(), what)?;
                Ok(ChangeSurface::measure(under))
            }
            TopologyChange::Insert {
                what,
                at_index,
                under,
            } => {
                self.aggregates
                    .hierarchy
                    .insert_at(under.clone(), at_index, what)?;
                Ok(ChangeSurface::measure(under))
            }
            TopologyChange::Reparent { what, under } => {
                self.reparenting
                    .apply(what, under, &mut self.aggregates, &self.layout_state)
            }
            TopologyChange::Remove(target) => {
                // A removed subtree may still hold pointer and/or keyboard focus. Clear pointer
                // focus and retarget keyboard focus to the parent before removal so the event
                // router is not left pointing at a removed node.
                self.unfocus_pointer_if_path_contains(&target, instance_manager)?;
                self.refocus_to_parent_if_path_contains(&target, instance_manager)?;
                self.remove_target(&target).map(ChangeSurface::measure)
            }
        }
    }

    /// Applies `change` to the configuration and marks the projects it touched, so that their shared
    /// column counts are refreshed once the transaction's changes have all run (ADR 0035).
    fn apply_project_change(&mut self, change: ConfigurationChange) -> Result<ChangeOutput> {
        let touched = change.touched_projects();
        let mcp_indicators = change.mcp_indicator_projects();
        let mut output = self.apply_configuration_change(change)?;
        output.surface.column_counts_invalid += touched;
        output.surface.mcp_indicators_invalid += mcp_indicators;
        Ok(output)
    }

    fn apply_configuration_change(&mut self, change: ConfigurationChange) -> Result<ChangeOutput> {
        match change {
            ConfigurationChange::AssignSlot {
                parent,
                slot,
                assignment,
            } => {
                let target = assignment.content();
                // The presenter is built from what the aggregate stored, not what
                // the payload claimed: the host comes back from the mutation, the
                // stored name is read from the aggregate (it may have been
                // re-indexed to stay unique).
                let host = self
                    .aggregates
                    .configuration
                    .assign_slot(parent, slot, assignment);
                self.insert_slot_presenter(target, host)?;
            }
            ConfigurationChange::ClearSlot { parent, slot } => {
                // A cleared project takes its whole subtree along, so every presenter below it
                // goes too. Its counts are dropped by the refresh, which finds it gone.
                let mut output = ChangeOutput::default();
                let removed = self.aggregates.configuration.clear_slot(parent, slot);
                for content in removed.content {
                    match content {
                        SlotContent::Launcher(launcher) => {
                            self.aggregates.launchers.remove(&launcher)?;
                        }
                        SlotContent::Project(project) => {
                            self.aggregates.projects.remove(&project)?;
                            output.surface.column_counts_invalid += project;
                        }
                    }
                }
                for root in removed.mcp_roots {
                    output.surface.mcp_roots_changed += root;
                }
                return Ok(output);
            }
            ConfigurationChange::MoveSlot { source, dest } => {
                let (source_parent, _) = source;
                let (dest_parent, _) = dest;
                if !self.aggregates.configuration.move_slot(source, dest) {
                    return Ok(ChangeOutput::default());
                }
                return Ok(ChangeOutput::measures(TargetSet::from_iter([
                    DesktopTarget::matrix(source_parent),
                    DesktopTarget::matrix(dest_parent),
                ])));
            }
            ConfigurationChange::Group {
                parent,
                slots,
                project,
                name,
            } => {
                self.aggregates
                    .configuration
                    .group_slots(parent, &slots, project, &name);
                self.insert_slot_presenter(SlotContent::Project(project), Some(parent))?;
            }
            ConfigurationChange::Ungroup { parent, slot } => {
                let project = self.aggregates.configuration.ungroup_slot(parent, slot);
                self.aggregates.projects.remove(&project)?;
                let mut output = ChangeOutput::measures(DesktopTarget::matrix(parent));
                output.surface.column_counts_invalid += project;
                return Ok(output);
            }
            ConfigurationChange::SwapSlots { parent, a, b } => {
                self.aggregates.configuration.swap_slots(parent, a, b);
                return Ok(ChangeOutput::measures(DesktopTarget::matrix(parent)));
            }
            ConfigurationChange::RelocateSlots { parent, landings } => {
                self.aggregates
                    .configuration
                    .relocate_slots(parent, &landings);
                return Ok(ChangeOutput::measures(DesktopTarget::matrix(parent)));
            }
            // The startup launcher is consumed at boot (`Setup`); the runtime model
            // does not retain the resolved launcher. The path is what persists, so
            // the aggregate records it and the document derives from it.
            ConfigurationChange::SetStartupPath(path) => {
                self.aggregates.configuration.set_startup_path(path);
            }
            ConfigurationChange::ConfigureMcp { project, mcp } => {
                self.aggregates.configuration.configure_mcp(project, mcp);
                let mut output = ChangeOutput::default();
                output.surface.mcp_roots_changed += project;
                return Ok(output);
            }
        }

        Ok(ChangeOutput::default())
    }

    /// Builds the presenter for newly assigned slot content. The host is the
    /// one `assign_slot` returned — no re-search over the aggregate. The stored
    /// name is read back from the aggregate, so the builder sees what was
    /// actually stored rather than what the change payload claimed.
    ///
    /// A launcher always has a hosting project (`None` is an invariant
    /// violation); the root project is hosted by the desktop itself and takes
    /// the desktop location instead of a matrix's.
    fn insert_slot_presenter(
        &mut self,
        content: SlotContent,
        host: Option<ProjectId>,
    ) -> Result<()> {
        match content {
            SlotContent::Launcher(id) => {
                let parent = host.context(format!("a launcher is hosted by a project: {id:?}"))?;
                let name = self.aggregates.configuration[id].name.clone();
                self.insert_launcher_presenter(parent, id, name)
            }
            SlotContent::Project(id) => self.insert_project_presenter(host, id),
        }
    }

    /// Inserts the projector presenter for a newly created project. The plan
    /// guarantees the id does not exist yet — the root is created by a parentless
    /// `AssignSlot`, and every nested project arrives via `AssignSlot` with a
    /// fresh id — so an existing presenter is an invariant violation and fails
    /// loudly here.
    fn insert_project_presenter(&mut self, host: Option<ProjectId>, id: ProjectId) -> Result<()> {
        // A nested project's scene node hangs under the matrix that hosts its slot:
        // its layout transform is relative to that matrix. The root project — with
        // no host — is placed in the desktop's own space.
        let location = match host.map(|parent| self.aggregates.project_matrix_location(parent)) {
            Some(location) => location,
            None => self.desktop_presenter.location.clone(),
        };
        let presenter = ProjectPresenter::new(
            id,
            &self.project_header_text(id),
            self.mcp_activation(id),
            location,
        );
        self.aggregates.projects.insert(id, presenter)
    }

    /// Inserts the launcher presenter for a newly assigned launcher. `AssignSlot`
    /// never re-assigns an existing launcher id, so an existing presenter is an
    /// invariant violation and fails loudly here.
    fn insert_launcher_presenter(
        &mut self,
        parent: ProjectId,
        id: LaunchProfileId,
        name: String,
    ) -> Result<()> {
        let matrix_location = self.aggregates.project_matrix_location(parent);
        let presenter = LauncherPresenter::new(
            matrix_location,
            id,
            name,
            self.mcp_activation(parent),
            massive_geometry::Size::default(),
        );
        self.aggregates.launchers.insert(id, presenter)
    }

    fn apply_instance_submission(
        &mut self,
        instance: InstanceId,
        submission: InstanceSubmission,
    ) -> Result<ChangeOutput> {
        let (changes, pacing) = submission.into_parts();
        let mut output = ChangeOutput::default();

        for change in changes.release() {
            output.combine(self.apply_instance_change(instance, change)?);
        }

        self.set_instance_pacing(instance, pacing);
        Ok(output)
    }

    fn apply_instance_change(
        &mut self,
        instance: InstanceId,
        change: InstanceChange,
    ) -> Result<ChangeOutput> {
        match change {
            InstanceChange::Scene(change) => {
                submit(change);
                Ok(ChangeOutput::default())
            }
            InstanceChange::CreateView(creation_info) => {
                let mut output = self.present_view(instance, &creation_info)?;
                output.measure(instance.into());

                // If this instance is currently focused and the new view is primary, make it
                // foreground so that the view is focused. Emitted as a follow-up change so the
                // focus transition (and its navigation-affinity reset) flows through change
                // application like every other focus change.
                if let (Some(FocusableTarget::Instance(focused_instance)), ViewRole::Primary) =
                    (self.keyboard_focus.focused(), &creation_info.role)
                    && focused_instance == instance
                {
                    output.changes += set_focus(
                        Some(FocusableTarget::View(instance, creation_info.id)),
                        KeyboardFocusReason::PromotePrimaryView,
                    );
                }
                Ok(output)
            }
            InstanceChange::DestroyView(id) => {
                let view_path: ViewPath = (instance, id).into();
                self.hide_view(view_path)
            }
            InstanceChange::View(view_id, command) => {
                let view_path: ViewPath = (instance, view_id).into();
                self.apply_view_change(view_path, command)?;
                Ok(ChangeOutput::default())
            }
            InstanceChange::Configuration(request) => {
                self.apply_configuration_request(instance, request)
            }
            // This makes sure that all pending Scene Changes from the Instance have been collected
            // before we drop the last ref the instance has to its parent location (which in turn
            // may push other deletes to the Scene).
            InstanceChange::End(_) => Ok(ChangeOutput::default()),
        }
    }

    fn apply_view_change(&mut self, view_path: ViewPath, change: ViewChange) -> Result<()> {
        // We can never be sure if the instance does exist here.
        if let Some(instance) = self.aggregates.instances.get_mut(&view_path.instance) {
            match change {
                ViewChange::Resize(_extends) => {
                    // Resize isn't supported yet.
                    todo!("View Resizes aren't supported yet");
                }
                ViewChange::SetTitle(title) => {
                    debug!("Setting title: {title}");
                    instance.set_view_title(view_path.view, title)?;
                }
                ViewChange::SetCursor(cursor) => {
                    debug!("Setting cursor: {cursor}");
                    instance.set_view_cursor(view_path.view, cursor)?;
                }
                ViewChange::SetImeArea(area) => {
                    instance.set_view_ime_area(view_path.view, area)?;
                }
            }
        }

        Ok(())
    }

    /// Builds the configuration command a CLI request stands for and plans it (ADR 0024). The
    /// request names content by names and target paths, relative to the instance the CLI runs in.
    fn apply_configuration_request(
        &self,
        instance: InstanceId,
        request: ConfigurationRequest,
    ) -> Result<ChangeOutput> {
        let current_project = self
            .aggregates
            .hierarchy
            .project_of_target(&instance.into());
        let command = match &request {
            ConfigurationRequest::Add { target, name } => {
                let Some(anchor) = self
                    .aggregates
                    .hierarchy
                    .slot_content_of_target(&instance.into())
                else {
                    warn!("{instance:?} sits in no slot to add next to");
                    return Ok(ChangeOutput::default());
                };
                ConfigurationCommand::AddNextTo {
                    anchor,
                    content: new_slot_assignment(*target, name.clone()),
                    focus: FocusIntent::Follow,
                }
            }
            ConfigurationRequest::Assign {
                target,
                name,
                column,
                row,
                under,
                shift,
            } => {
                let parent = match under {
                    Some(path) => self
                        .aggregates
                        .configuration
                        .resolve_project_path(current_project, path),
                    None => Some(current_project),
                };
                let Some(parent) = parent else {
                    warn!("Project path '{under:?}' does not resolve");
                    return Ok(ChangeOutput::default());
                };
                ConfigurationCommand::AssignSlot {
                    parent: Some(parent),
                    slot: SlotPosition {
                        column: *column,
                        row: *row,
                    },
                    content: new_slot_assignment(*target, Some(name.clone())),
                    shift: *shift,
                }
            }
            ConfigurationRequest::Remove { target, name } => {
                let content = match target {
                    ConfigurationTarget::Project => {
                        let project = match name {
                            Some(name) => {
                                let Some(project) = self
                                    .aggregates
                                    .configuration
                                    .nearest_project(name, self.focused_project())
                                else {
                                    warn!("Project '{name}' not found");
                                    return Ok(ChangeOutput::default());
                                };
                                project
                            }
                            None => current_project,
                        };
                        if project == ProjectId::ROOT {
                            warn!("The root project cannot be removed");
                            return Ok(ChangeOutput::default());
                        }
                        SlotContent::Project(project)
                    }
                    ConfigurationTarget::Launcher => {
                        let launcher = match name {
                            Some(name) => {
                                let Some(launcher) =
                                    self.aggregates.configuration.nearest_launcher(
                                        current_project,
                                        name,
                                        self.focused_launcher(),
                                    )
                                else {
                                    warn!("Launcher '{name}' not found in the current project");
                                    return Ok(ChangeOutput::default());
                                };
                                launcher
                            }
                            None => self.aggregates.hierarchy.launcher_of_instance(instance),
                        };
                        SlotContent::Launcher(launcher)
                    }
                };
                let Some((parent, slot)) = self.aggregates.configuration.slot_of_content(content)
                else {
                    return Ok(ChangeOutput::default());
                };
                ConfigurationCommand::ClearSlot {
                    parent,
                    slot,
                    shift: SlotShift::default(),
                }
            }
            ConfigurationRequest::MoveLauncher { direction } => {
                let launcher = self.aggregates.hierarchy.launcher_of_instance(instance);
                let Some((parent, slot)) = self.aggregates.configuration.slot_of_content(launcher)
                else {
                    warn!("The focused launcher has no slot");
                    return Ok(ChangeOutput::default());
                };
                let Some(next_slot) = slot.neighbor(*direction) else {
                    warn!(
                        "Ignoring {direction:?} launcher move from slot ({}, {})",
                        slot.column, slot.row,
                    );
                    return Ok(ChangeOutput::default());
                };

                // Onto an assigned neighbor, the move exchanges the two, so the launcher always
                // moves by one slot.
                ConfigurationCommand::MoveSlot {
                    source: (parent, slot),
                    dest: (parent, next_slot),
                }
            }
            ConfigurationRequest::PushLauncher { direction } => {
                let launcher = self.aggregates.hierarchy.launcher_of_instance(instance);
                let Some((parent, slot)) = self.aggregates.configuration.slot_of_content(launcher)
                else {
                    warn!("The focused launcher has no slot");
                    return Ok(ChangeOutput::default());
                };
                ConfigurationCommand::PushSlot {
                    parent,
                    slot,
                    direction: *direction,
                }
            }
            ConfigurationRequest::SetStartup { path } => {
                let path = match path {
                    Some(path) => {
                        let Some(launcher) = self
                            .aggregates
                            .configuration
                            .resolve_launcher_path(current_project, path)
                        else {
                            warn!("Startup path '{path}' does not resolve");
                            return Ok(ChangeOutput::default());
                        };
                        Some(
                            self.aggregates
                                .configuration
                                .launcher_target_path(launcher)?,
                        )
                    }
                    None => None,
                };
                ConfigurationCommand::SetStartupPath(path)
            }
            ConfigurationRequest::ConfigureMcp { path, mcp } => {
                let configuration = &self.aggregates.configuration;
                match security::configuring_mcp(
                    configuration.mcp_root_of(current_project).is_some(),
                ) {
                    ConfiguringMcp::Allow => {}
                    refusal => {
                        warn!("Refused to configure MCP: {refusal:?}");
                        return Ok(ChangeOutput::default());
                    }
                }
                let project = match path {
                    Some(path) => configuration.resolve_project_path(current_project, path),
                    None => Some(current_project),
                };
                let Some(project) = project else {
                    warn!("Project path '{path:?}' does not resolve");
                    return Ok(ChangeOutput::default());
                };
                ConfigurationCommand::ConfigureMcp {
                    project,
                    mcp: mcp.clone(),
                }
            }
            ConfigurationRequest::Resize { size_px } => {
                let changes = DesktopChange::ResizeAll((*size_px).into()).into();

                return Ok(ChangeOutput::changes(changes));
            }
            ConfigurationRequest::Undo => todo!(),
            ConfigurationRequest::Redo => todo!(),
        };
        Ok(ChangeOutput::changes(changes_of(
            self.configuration_planning().plan(command)?,
        )))
    }

    fn focused_project(&self) -> Option<ProjectId> {
        let focused = self.focused_target()?;
        let project = self.aggregates.hierarchy.project_of_target(&focused);
        self.aggregates
            .configuration
            .project(project)
            .map(|_| project)
    }

    fn focused_launcher(&self) -> Option<LaunchProfileId> {
        let focused = self.focused_target()?;
        let launcher = self.aggregates.hierarchy.launcher_of_target(&focused)?;
        self.aggregates
            .configuration
            .launcher(launcher)
            .map(|_| launcher)
    }

    /// The layout targets a change to `instance`'s presentation must measure:
    /// the instance itself plus its primary view, when it has one. The view's
    /// *measurement* (panel vs window size) and its placement both depend on
    /// Full Screen Mode, so a toggle must invalidate both.
    fn instance_view_targets(&self, instance: InstanceId) -> Vec<DesktopTarget> {
        let mut targets = vec![instance.into(), DesktopTarget::title_bar(instance)];
        if let Some(view) = self
            .aggregates
            .instances
            .get(&instance)
            .and_then(|presenter| presenter.primary_view_id())
        {
            targets.push(DesktopTarget::view(instance, view));
        }
        targets
    }
}

/// New, empty slot content of the requested kind, named `name` or by the kind's default.
fn new_slot_assignment(target: ConfigurationTarget, name: Option<String>) -> SlotAssignment {
    match target {
        ConfigurationTarget::Project => SlotAssignment::Project {
            id: ProjectId::new(),
            name: name.unwrap_or(DEFAULT_NEW_PROJECT_NAME.to_string()),
        },
        ConfigurationTarget::Launcher => SlotAssignment::Launcher {
            id: LaunchProfileId::new(),
            profile: LaunchProfile {
                name: name.unwrap_or(DEFAULT_NEW_LAUNCHER_NAME.to_string()),
                mode: LauncherMode::Visor,
                params: Default::default(),
                full_screen_mode: Default::default(),
            },
        },
    }
}

/// The changes of a planned command, or none for a refused one. The user is not told why nothing
/// happens yet, so the refusal is logged.
fn changes_of(planned: Planned) -> Changes {
    match planned {
        Planned::Changes(changes) => changes,
        Planned::Refused(refusal) => {
            warn!("Refused: {refusal}");
            Changes::Empty
        }
    }
}

/// A command a desktop policy guard refused is not issued (ADR 0029).
fn refused(refusal: impl fmt::Debug) -> Option<ConfigurationCommand> {
    // The user is not told why nothing happens yet. Telling them needs the notifications described
    // in `docs/desktop-notifications-plan.md`.
    debug!("Refused: {refusal:?}");
    None
}
