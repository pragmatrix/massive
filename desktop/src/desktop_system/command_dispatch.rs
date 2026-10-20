use anyhow::{Context, Result, bail};
use log::{debug, warn};
use serde_json::json;

use super::change::set_focus;
use super::grouping;
use super::reparenting::Reparenting;
use super::change::{
    Changes, ConfigurationChange, DesktopChange, InstancePresentation, ToggleFullScreenModeTarget,
    TopologyChange,
};
use super::{
    ChangeSurface, ConfigurationCommand, DesktopCommand, DesktopSystem, DesktopTarget,
    FocusableTarget, InstancePart, KeyboardFocusReason, ProjectPart, ZoomLevel, view_size,
};
use crate::desktop_system::change_surface::TargetSet;
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::instance_presenter::{InstanceKind, InstanceRoot};
use crate::projects::{
    DEFAULT_NEW_LAUNCHER_NAME, DEFAULT_NEW_PROJECT_NAME, LaunchProfile, LaunchProfileId,
    LauncherMode, LauncherPresenter, MatrixPlacement, ProjectId, ProjectPresenter, SlotAssignment,
    SlotContent,
};

use massive_applications::prelude::*;
use massive_applications::{
    ConfigurationRequest, ConfigurationTarget, CreationMode, InstanceChange, InstanceId,
    InstanceSubmission, MoveDirection, SlotShift, ViewChange, ViewEvent, ViewRole,
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
    /// Plan the execution of a command.
    pub fn plan(&self, command: DesktopCommand) -> Result<Changes> {
        match command {
            DesktopCommand::Configuration(configuration_command) => {
                self.plan_configuration(configuration_command)
            }
            DesktopCommand::StartInstance {
                launcher,
                instance,
                root,
                parameters,
                kind,
            } => {
                let originator_instance = self.focused_path().instance();
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
                        what: DesktopTarget::Instance(instance, InstancePart::TitleBar),
                        under: instance.into(),
                        after: None,
                    }),
                ];
                changes <<= DesktopChange::SetZoomLevel(ZoomLevel::Focus);
                changes += set_focus(
                    Some(FocusableTarget::Instance(instance)),
                    KeyboardFocusReason::PresentInstance,
                );

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
            DesktopCommand::AddLauncher => {
                let Some(focused) = self.focused_target() else {
                    return Ok(Changes::Empty);
                };
                self.plan_add_next_to(
                    &focused,
                    new_slot_assignment(ConfigurationTarget::Launcher, None),
                )
            }
            DesktopCommand::RemoveSlot(content) => {
                // The root project sits in no slot, so it cannot be removed.
                let Some((parent, placement)) =
                    self.aggregates.configuration.slot_of_content(content)
                else {
                    return Ok(Changes::Empty);
                };
                if self
                    .aggregates
                    .hierarchy
                    .has_instances_below(&content.target())
                {
                    // The user is not told why nothing happens yet. Telling them needs the
                    // notifications described in `docs/desktop-notifications-plan.md`.
                    debug!("Keeping {content:?}, because instances run below it");
                    return Ok(Changes::Empty);
                }
                self.plan_configuration(ConfigurationCommand::ClearSlot {
                    parent,
                    placement,
                    shift: SlotShift::default(),
                })
            }
            DesktopCommand::GroupSlot => {
                let Some(content) = self
                    .focused_target()
                    .and_then(|focused| self.aggregates.hierarchy.slot_content_of_target(&focused))
                else {
                    warn!("The focus sits in no slot to group");
                    return Ok(Changes::Empty);
                };
                Ok(grouping::plan(content, &self.aggregates.configuration))
            }
            DesktopCommand::UngroupSlot => {
                // Only a focused nested project is ungrouped, so the user sees what is unpacked.
                let Some(FocusableTarget::Project(project)) = self.keyboard_focus.focused() else {
                    return Ok(Changes::Empty);
                };
                if project == ProjectId::ROOT {
                    return Ok(Changes::Empty);
                }
                let configuration = &self.aggregates.configuration;
                if configuration.first_slot(project).is_none() {
                    // An empty project has nothing to unpack, so ungrouping just removes it.
                    let (parent, placement) = configuration
                        .slot_of_content(project)
                        .expect("a nested project sits in a slot");
                    return self.plan_configuration(ConfigurationCommand::ClearSlot {
                        parent,
                        placement,
                        shift: SlotShift::Keep,
                    });
                }
                Ok(grouping::plan_ungroup(project, configuration))
            }
            DesktopCommand::Navigate(direction) => self.plan_navigate(direction),
            DesktopCommand::Zoom(zoom) => Ok(self.zoom_navigation().plan_zoom(zoom)),
            DesktopCommand::ToggleFullScreen => {
                if self.window_state.is_fullscreen
                    && self.focused_path().instance().is_some()
                    && self.zoom_navigation().is_fully_zoomed_in()
                {
                    return self.plan_toggle_full_screen_mode();
                }
                Ok(DesktopChange::ToggleWindowFullScreen.into())
            }
        }
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

    fn plan_configuration(&self, command: ConfigurationCommand) -> Result<Changes> {
        let mut changes = Changes::Empty;
        match command {
            ConfigurationCommand::AssignSlot {
                parent,
                placement,
                content,
                shift,
            } => {
                let Some(parent) = parent else {
                    return self.plan_root_creation(placement, content);
                };
                // The parsed aggregate already has boot slots, while the topology
                // starts empty; only displace occupants that are live in the scene.
                let assigned = self
                    .aggregates
                    .configuration
                    .content_at(parent, placement)
                    .filter(|previous| self.aggregates.hierarchy.exists(&previous.target()));
                if assigned.is_some() {
                    match shift {
                        SlotShift::Shift => {
                            changes +=
                                self.slot_shift_sequence(parent, placement, MoveDirection::Right)?;
                        }
                        SlotShift::Keep => {
                            changes += self.plan_clear_slot(parent, placement, SlotShift::Keep)?;
                        }
                    }
                }

                changes <<= ConfigurationChange::AssignSlot {
                    parent: Some(parent),
                    placement,
                    assignment: content.clone(),
                };
                let under = DesktopTarget::Project(parent, ProjectPart::Matrix);
                match content.content() {
                    SlotContent::Project(project) => changes += project_topology(project, under),
                    SlotContent::Launcher(launcher) => {
                        changes <<= TopologyChange::Add {
                            what: DesktopTarget::Launcher(launcher),
                            under,
                            after: None,
                        }
                    }
                }
            }
            ConfigurationCommand::ClearSlot {
                parent,
                placement,
                shift,
            } => {
                changes += self.plan_clear_slot(parent, placement, shift)?;
            }
            ConfigurationCommand::MoveSlot { source, dest } => {
                if self.aggregates.configuration.can_move_slot(source, dest) {
                    changes += Reparenting::plan(source, dest, &self.aggregates.configuration);
                    changes <<= ConfigurationChange::MoveSlot { source, dest };
                }
            }
            ConfigurationCommand::SetStartupPath(path) => {
                changes <<= ConfigurationChange::SetStartupPath(path)
            }
        }

        Ok(changes)
    }

    /// Creating the root is a parentless assignment of [`ProjectId::ROOT`] under
    /// the `Desktop` target: the root is hosted by no project and lives in no
    /// slot, so `placement` is carried through but never slotted.
    fn plan_root_creation(
        &self,
        placement: MatrixPlacement,
        content: SlotAssignment,
    ) -> Result<Changes> {
        let (id, name) = match content {
            SlotAssignment::Project { id, name } => (id, name),
            // Only boot plans a parentless assignment, and it only creates the root.
            SlotAssignment::Launcher { .. } => {
                panic!("a launcher is always assigned into a project's slot")
            }
        };
        if id != ProjectId::ROOT
            || self.aggregates.hierarchy.exists(&DesktopTarget::Project(
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
            placement,
            assignment: SlotAssignment::Project { id, name },
        };
        changes += project_topology(id, DesktopTarget::Desktop);
        Ok(changes)
    }

    fn slot_shift_sequence(
        &self,
        project: ProjectId,
        placement: MatrixPlacement,
        direction: MoveDirection,
    ) -> Result<Changes> {
        let mut changes = Changes::Empty;
        for (source, dest) in self
            .aggregates
            .configuration
            .shifted_slots(project, placement, direction)?
        {
            changes <<= ConfigurationChange::MoveSlot {
                source: (project, source),
                dest: (project, dest),
            };
        }
        Ok(changes)
    }

    fn plan_clear_slot(
        &self,
        parent: ProjectId,
        placement: MatrixPlacement,
        shift: SlotShift,
    ) -> Result<Changes> {
        let Some(content) = self.aggregates.configuration.content_at(parent, placement) else {
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
        changes <<= ConfigurationChange::ClearSlot { parent, placement };

        if shift == SlotShift::Shift {
            for (source, dest) in self
                .aggregates
                .configuration
                .shifted_left_slots(parent, placement)
            {
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
        if !self.focused_path().contains(&content.target()) {
            return Changes::Empty;
        }
        let replacement = self.slot_removal_focus(content);
        set_focus(Some(replacement), KeyboardFocusReason::InputTransition)
    }

    fn plan_remove_launcher_instances(&self, launcher: LaunchProfileId) -> Changes {
        let mut changes = Changes::Empty;
        for instance in self.aggregates.hierarchy.launcher_instances(launcher) {
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
        for slot in self.aggregates.configuration.slots_ordered(project) {
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
            DesktopChange::SetInputFocus(input_focus) => {
                self.keyboard_focus.set_input_focus(input_focus);
            }
            DesktopChange::SetZoomLevel(level) => {
                if self.zoom_level != level {
                    self.zoom_level = level;

                    let mut output = ChangeOutput::default();
                    if let Some(focused) = self.focused_target() {
                        output.measure(focused);
                    }
                    return Ok(output);
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
                let mut output = ChangeOutput::default();
                output.surface.configuration_changed = true;
                for changed_target in changed_targets {
                    output.measure(changed_target);
                }
                return Ok(output);
            }
            DesktopChange::WindowResized(window_state) => {
                self.window_state = window_state;
                let mut output = ChangeOutput::default();
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
                let mut output = self.apply_project_change(project_change)?;
                output.surface.configuration_changed = true;
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

    fn apply_project_change(&mut self, change: ConfigurationChange) -> Result<ChangeOutput> {
        match change {
            ConfigurationChange::AssignSlot {
                parent,
                placement,
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
                    .assign_slot(parent, placement, assignment);
                self.insert_slot_presenter(target, host)?;
            }
            ConfigurationChange::ClearSlot { parent, placement } => {
                if let Some(content) = self.aggregates.configuration.content_at(parent, placement) {
                    match content {
                        SlotContent::Launcher(launcher) => {
                            self.aggregates.launchers.remove(&launcher)?;
                        }
                        SlotContent::Project(project) => {
                            self.aggregates.projects.remove(&project)?;
                        }
                    }
                }
                self.aggregates.configuration.clear_slot(parent, placement);
            }
            ConfigurationChange::MoveSlot { source, dest } => {
                let (source_parent, _) = source;
                let (dest_parent, _) = dest;
                if !self.aggregates.configuration.move_slot(source, dest) {
                    return Ok(ChangeOutput::default());
                }
                let mut output = ChangeOutput::default();
                output.measure(DesktopTarget::Project(source_parent, ProjectPart::Matrix));
                output.measure(DesktopTarget::Project(dest_parent, ProjectPart::Matrix));
                return Ok(output);
            }
            ConfigurationChange::Group {
                parent,
                placement,
                project,
                name,
            } => {
                self.aggregates
                    .configuration
                    .group_slot(parent, placement, project, &name);
                self.insert_slot_presenter(SlotContent::Project(project), Some(parent))?;
            }
            ConfigurationChange::Ungroup { parent, placement } => {
                let project = self
                    .aggregates
                    .configuration
                    .ungroup_slot(parent, placement);
                self.aggregates.projects.remove(&project)?;
                let mut output = ChangeOutput::default();
                output.measure(DesktopTarget::Project(parent, ProjectPart::Matrix));
                return Ok(output);
            }
            ConfigurationChange::SwapSlots { parent, a, b } => {
                self.aggregates.configuration.swap_slots(parent, a, b);
                let mut output = ChangeOutput::default();
                output.measure(DesktopTarget::Project(parent, ProjectPart::Matrix));
                return Ok(output);
            }
            // The startup launcher is consumed at boot (`Setup`); the runtime model
            // does not retain the resolved launcher. The path is what persists, so
            // the aggregate records it and the document derives from it.
            ConfigurationChange::SetStartupPath(path) => {
                self.aggregates.configuration.set_startup_path(path);
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
            SlotContent::Project(id) => {
                let name = self.aggregates.configuration[id].name.clone();
                self.insert_project_presenter(host, id, name)
            }
        }
    }

    /// Inserts the projector presenter for a newly created project. The plan
    /// guarantees the id does not exist yet — the root is created by a parentless
    /// `AssignSlot`, and every nested project arrives via `AssignSlot` with a
    /// fresh id — so an existing presenter is an invariant violation and fails
    /// loudly here.
    fn insert_project_presenter(
        &mut self,
        host: Option<ProjectId>,
        id: ProjectId,
        name: String,
    ) -> Result<()> {
        // A nested project's scene node hangs under the matrix that hosts its slot:
        // its layout transform is relative to that matrix. The root project — with
        // no host — is placed in the desktop's own space.
        let location = match host.map(|parent| self.aggregates.project_matrix_location(parent)) {
            Some(location) => location,
            None => self.desktop_presenter.location.clone(),
        };
        let presenter = ProjectPresenter::new(id, name, location);
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
        let presenter =
            LauncherPresenter::new(matrix_location, id, name, massive_geometry::Size::default());
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

    fn apply_configuration_request(
        &self,
        instance: InstanceId,
        request: ConfigurationRequest,
    ) -> Result<ChangeOutput> {
        let current_project = self
            .aggregates
            .hierarchy
            .project_of_target(&instance.into());
        match &request {
            ConfigurationRequest::Add { target, name } => {
                let changes = self.plan_add_next_to(
                    &instance.into(),
                    new_slot_assignment(*target, name.clone()),
                )?;
                Ok(ChangeOutput::changes(changes))
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
                let content = new_slot_assignment(*target, Some(name.clone()));
                Ok(ChangeOutput::changes(self.plan_configuration(
                    ConfigurationCommand::AssignSlot {
                        parent: Some(parent),
                        placement: MatrixPlacement {
                            column: *column,
                            row: *row,
                        },
                        content,
                        shift: *shift,
                    },
                )?))
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
                let Some((parent, placement)) =
                    self.aggregates.configuration.slot_of_content(content)
                else {
                    return Ok(ChangeOutput::default());
                };
                Ok(ChangeOutput::changes(self.plan_configuration(
                    ConfigurationCommand::ClearSlot {
                        parent,
                        placement,
                        shift: SlotShift::default(),
                    },
                )?))
            }
            ConfigurationRequest::MoveLauncher { direction } => {
                let launcher = self.aggregates.hierarchy.launcher_of_instance(instance);
                let Some((parent, current_placement)) =
                    self.aggregates.configuration.slot_of_content(launcher)
                else {
                    warn!("The focused launcher has no matrix placement");
                    return Ok(ChangeOutput::default());
                };
                let Some(placement) = current_placement.moved_placement(*direction) else {
                    warn!(
                        "Ignoring {direction:?} launcher move from matrix position ({}, {})",
                        current_placement.column, current_placement.row,
                    );
                    return Ok(ChangeOutput::default());
                };

                let source = (parent, current_placement);
                let destination = (parent, placement);
                if !self
                    .aggregates
                    .configuration
                    .can_move_slot(source, destination)
                {
                    return Ok(ChangeOutput::default());
                }

                let mut changes = Changes::Empty;
                if self
                    .aggregates
                    .configuration
                    .content_at(parent, placement)
                    .is_some()
                {
                    changes <<= ConfigurationChange::SwapSlots {
                        parent,
                        a: current_placement,
                        b: placement,
                    };
                } else {
                    changes += self.plan_configuration(ConfigurationCommand::MoveSlot {
                        source,
                        dest: destination,
                    })?;
                }
                Ok(ChangeOutput::changes(changes))
            }
            ConfigurationRequest::PushLauncher { direction } => {
                let launcher = self.aggregates.hierarchy.launcher_of_instance(instance);
                let Some((parent, current_placement)) =
                    self.aggregates.configuration.slot_of_content(launcher)
                else {
                    warn!("The focused launcher has no matrix placement");
                    return Ok(ChangeOutput::default());
                };
                match self.slot_shift_sequence(parent, current_placement, *direction) {
                    Ok(changes) => Ok(ChangeOutput::changes(changes)),
                    Err(_) => {
                        warn!(
                            "Ignoring {direction:?} launcher push from matrix position ({}, {})",
                            current_placement.column, current_placement.row,
                        );
                        Ok(ChangeOutput::default())
                    }
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
                                .launcher_address_path(launcher)?,
                        )
                    }
                    None => None,
                };
                Ok(ChangeOutput::changes(self.plan_configuration(
                    ConfigurationCommand::SetStartupPath(path),
                )?))
            }
            ConfigurationRequest::Resize { size_px } => {
                let changes = DesktopChange::ResizeAll((*size_px).into()).into();

                Ok(ChangeOutput::changes(changes))
            }
            ConfigurationRequest::Undo => todo!(),
            ConfigurationRequest::Redo => todo!(),
        }
    }

    /// Assigns `content` right of the slot `anchor` sits in, shifting the row, and focuses it, so
    /// new content appears where the user works (Cmd+N and the CLI `add`).
    fn plan_add_next_to(&self, anchor: &DesktopTarget, content: SlotAssignment) -> Result<Changes> {
        let Some(anchor) = self.aggregates.hierarchy.slot_content_of_target(anchor) else {
            warn!("{anchor:?} sits in no slot to add next to");
            return Ok(Changes::Empty);
        };
        let (parent, placement) = self
            .aggregates
            .configuration
            .slot_of_content(anchor)
            .expect("slot content of a live target sits in a slot");

        let focus = content.content().focusable_target();
        let mut changes = self.plan_configuration(ConfigurationCommand::AssignSlot {
            parent: Some(parent),
            placement: MatrixPlacement {
                column: placement.column + 1,
                row: placement.row,
            },
            content,
            shift: SlotShift::default(),
        })?;
        changes += set_focus(Some(focus), KeyboardFocusReason::AddSlotContent);
        Ok(changes)
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
        let mut targets = vec![
            instance.into(),
            DesktopTarget::Instance(instance, InstancePart::TitleBar),
        ];
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

/// Adds a project and its header and matrix under the hosting target.
pub fn project_topology(project: ProjectId, under: DesktopTarget) -> Changes {
    let mut changes = Changes::Empty;
    changes <<= TopologyChange::Add {
        what: project.into(),
        under,
        after: None,
    };
    changes <<= TopologyChange::AddNested {
        what: [
            DesktopTarget::Project(project, ProjectPart::Header),
            DesktopTarget::Project(project, ProjectPart::Matrix),
        ]
        .into(),
        under: project.into(),
    };
    changes
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
