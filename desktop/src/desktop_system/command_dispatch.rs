use anyhow::{Context, Result, bail};
use log::{debug, warn};
use serde_json::json;

use super::change::set_focus;
use super::change::{
    Changes, ConfigurationChange, DesktopChange, InstancePresentation, ToggleFullScreenModeTarget,
    TopologyChange,
};
use super::{
    ChangeSurface, DesktopCommand, DesktopSystem, DesktopTarget, KeyboardFocusReason,
    ProjectCommand, ZoomLevel, view_size,
};
use crate::desktop_system::change_surface::TargetSet;
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::instance_presenter::{InstanceKind, InstanceRoot};
use crate::projects::{
    DEFAULT_NEW_LAUNCHER_NAME, LaunchProfile, LaunchProfileId, LauncherMode, LauncherPresenter,
    MatrixPlacement, ProjectId, ProjectPresenter, SlotAssignment, SlotContent,
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
            DesktopCommand::Project(project_command) => self.plan_project(project_command),
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
                        self.title_bar.height,
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
                        what: DesktopTarget::InstanceTitleBar(instance),
                        under: DesktopTarget::Instance(instance),
                        after: None,
                    }),
                ];
                changes <<= DesktopChange::SetZoomLevel(ZoomLevel::Focus);
                changes += set_focus(
                    Some(DesktopTarget::Instance(instance)),
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
                let replacement_focus = self.event_router.keyboard_focus().and_then(|focused| {
                    self.aggregates
                        .hierarchy
                        .resolve_replacement_focus_for_stopping_instance(focused, instance)
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
            DesktopCommand::Zoom(zoom) => Ok(self.plan_zoom(zoom)),
            DesktopCommand::ToggleFullScreen => {
                if self.window_state.is_fullscreen
                    && self.focused_path().instance().is_some()
                    && self
                        .event_router
                        .keyboard_focus()
                        .is_some_and(|focused| self.is_fully_zoomed_in(focused, self.zoom_level))
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

        match self.event_router.keyboard_focus() {
            Some(DesktopTarget::Launcher(launcher))
                if self
                    .aggregates
                    .hierarchy
                    .launcher_instances(*launcher)
                    .next()
                    .is_some() =>
            {
                changes <<= DesktopChange::ToggleFullScreenMode(
                    ToggleFullScreenModeTarget::Launcher(*launcher),
                );
            }
            Some(target @ (DesktopTarget::Instance(_) | DesktopTarget::View(_))) => {
                let instance = match target {
                    DesktopTarget::Instance(instance) => Some(*instance),
                    DesktopTarget::View(_) => self.aggregates.hierarchy.instance_of_target(target),
                    _ => None,
                };
                let toggle = instance.map(|instance| {
                    // An assistant toggles its own temporary mode; a base
                    // instance toggles its launcher's mode (ADR 0014).
                    if self
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
                    }
                });
                if let Some(toggle) = toggle {
                    changes <<= DesktopChange::ToggleFullScreenMode(toggle);
                }
            }
            _ => {}
        }

        Ok(changes)
    }

    fn plan_project(&self, command: ProjectCommand) -> Result<Changes> {
        let mut changes = Changes::Empty;
        match command {
            ProjectCommand::AssignSlot {
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
                let under = DesktopTarget::ProjectMatrix(parent);
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
            ProjectCommand::ClearSlot {
                parent,
                placement,
                shift,
            } => {
                changes += self.plan_clear_slot(parent, placement, shift)?;
            }
            ProjectCommand::MoveSlot { source, dest } => {
                if self.aggregates.configuration.can_move_slot(source, dest) {
                    changes <<= ConfigurationChange::MoveSlot { source, dest };
                }
            }
            ProjectCommand::SetStartupPath(path) => {
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
            || self
                .aggregates
                .hierarchy
                .exists(&DesktopTarget::Project(ProjectId::ROOT))
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
                changes <<= TopologyChange::Remove(DesktopTarget::Project(project));
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
        let Some(focused) = self.event_router.keyboard_focus() else {
            return Changes::Empty;
        };
        if !self
            .aggregates
            .hierarchy
            .path_contains_target(Some(focused), &content.target())
        {
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
                    changes <<= TopologyChange::Remove(DesktopTarget::Project(nested));
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
                let previous_focus = self.event_router.keyboard_focus().cloned();
                self.focus(target.as_ref(), instance_manager)?;
                let current_focus = self.event_router.keyboard_focus().cloned();

                let mut output = ChangeOutput::default();
                output.focus_changed(previous_focus, current_focus);

                return Ok(output);
            }
            DesktopChange::SetNavigationAffinity(column_affinity) => {
                self.navigation_control
                    .commit_column_affinity(column_affinity);
            }
            DesktopChange::SetZoomLevel(level) => {
                if self.zoom_level != level {
                    self.zoom_level = level;

                    let mut output = ChangeOutput::default();
                    if let Some(focused) = self.event_router.keyboard_focus() {
                        output.measure(focused.clone());
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
                let previous_focus = self.event_router.keyboard_focus().cloned();
                // Design: That's somewhat unexpected here, that `apply_topology_change` changes
                // focus. Can we make this more obvious? We should combine the `instance_manager`
                // side effects perhaps.
                let measure_target = self.apply_topology_change(change, instance_manager)?;
                let current_focus = self.event_router.keyboard_focus().cloned();

                let mut output = ChangeOutput::measures(measure_target);
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
    ) -> Result<DesktopTarget> {
        match change {
            TopologyChange::Add { what, under, after } => {
                if let Some(after) = after {
                    // Design: `under` can be resolved via `after`!
                    self.aggregates.hierarchy.add_after(after, what)?;
                } else {
                    self.aggregates.hierarchy.add(under.clone(), what)?;
                }
                Ok(under)
            }
            TopologyChange::AddNested { what, under } => {
                self.aggregates.hierarchy.add_nested(under.clone(), what)?;
                Ok(under)
            }
            TopologyChange::Insert {
                what,
                at_index,
                under,
            } => {
                self.aggregates
                    .hierarchy
                    .insert_at(under.clone(), at_index, what)?;
                Ok(under)
            }
            TopologyChange::Remove(target) => {
                // A removed subtree may still hold pointer and/or keyboard focus. Clear pointer
                // focus and retarget keyboard focus to the parent before removal so the event
                // router is not left pointing at a removed node.
                self.unfocus_pointer_if_path_contains(&target, instance_manager)?;
                self.refocus_to_parent_if_path_contains(&target, instance_manager)?;
                self.remove_target(&target)
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
                output.measure(DesktopTarget::ProjectMatrix(source_parent));
                output.measure(DesktopTarget::ProjectMatrix(dest_parent));
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
        let presenter = ProjectPresenter::new(name, location);
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
                output.measure(DesktopTarget::Instance(instance));

                // If this instance is currently focused and the new view is primary, make it
                // foreground so that the view is focused. Emitted as a follow-up change so the
                // focus transition (and its navigation-affinity reset) flows through change
                // application like every other focus change.
                if let (Some(DesktopTarget::Instance(focused_instance)), ViewRole::Primary) =
                    (self.event_router.keyboard_focus(), &creation_info.role)
                    && *focused_instance == instance
                {
                    output.changes += set_focus(
                        Some(DesktopTarget::View(creation_info.id)),
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
            ConfigurationRequest::AddLauncher => {
                let launcher = self.aggregates.hierarchy.launcher_of_instance(instance);
                let Some((parent, current_placement)) =
                    self.aggregates.configuration.slot_of_content(launcher)
                else {
                    warn!("The focused launcher has no matrix placement");
                    return Ok(ChangeOutput::default());
                };

                let changes = self.plan_project(ProjectCommand::AssignSlot {
                    parent: Some(parent),
                    placement: MatrixPlacement {
                        column: current_placement.column + 1,
                        row: current_placement.row,
                    },
                    content: SlotAssignment::Launcher {
                        id: LaunchProfileId::new(),
                        profile: LaunchProfile {
                            name: DEFAULT_NEW_LAUNCHER_NAME.to_string(),
                            mode: LauncherMode::Visor,
                            params: Default::default(),
                            full_screen_mode: Default::default(),
                        },
                    },
                    shift: SlotShift::default(),
                })?;
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
                let content = match target {
                    ConfigurationTarget::Project => SlotAssignment::Project {
                        id: ProjectId::new(),
                        name: name.clone(),
                    },
                    ConfigurationTarget::Launcher => SlotAssignment::Launcher {
                        id: LaunchProfileId::new(),
                        profile: LaunchProfile {
                            name: name.clone(),
                            mode: LauncherMode::Visor,
                            params: Default::default(),
                            full_screen_mode: Default::default(),
                        },
                    },
                };
                Ok(ChangeOutput::changes(self.plan_project(
                    ProjectCommand::AssignSlot {
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
                Ok(ChangeOutput::changes(self.plan_project(
                    ProjectCommand::ClearSlot {
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
                    let temporary = self.next_free_root_slot(parent);
                    changes <<= ConfigurationChange::MoveSlot {
                        source,
                        dest: (parent, temporary),
                    };
                    changes <<= ConfigurationChange::MoveSlot {
                        source: destination,
                        dest: source,
                    };
                    changes <<= ConfigurationChange::MoveSlot {
                        source: (parent, temporary),
                        dest: destination,
                    };
                } else {
                    changes += self.plan_project(ProjectCommand::MoveSlot {
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
                Ok(ChangeOutput::changes(
                    self.plan_project(ProjectCommand::SetStartupPath(path))?,
                ))
            }
            ConfigurationRequest::Resize { size_px } => {
                let changes = DesktopChange::ResizeAll((*size_px).into()).into();

                Ok(ChangeOutput::changes(changes))
            }
            ConfigurationRequest::Undo => todo!(),
            ConfigurationRequest::Redo => todo!(),
        }
    }

    fn next_free_root_slot(&self, project: ProjectId) -> MatrixPlacement {
        let mut row = 0;
        while self
            .aggregates
            .configuration
            .content_at(project, MatrixPlacement { column: 0, row })
            .is_some()
        {
            row += 1;
        }
        MatrixPlacement { column: 0, row }
    }

    fn focused_project(&self) -> Option<ProjectId> {
        let focused = self.event_router.keyboard_focus()?;
        let project = self.aggregates.hierarchy.project_of_target(focused);
        self.aggregates
            .configuration
            .project(project)
            .map(|_| project)
    }

    fn focused_launcher(&self) -> Option<LaunchProfileId> {
        let focused = self.event_router.keyboard_focus()?;
        let launcher = self.aggregates.hierarchy.launcher_of_target(focused)?;
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
            DesktopTarget::Instance(instance),
            DesktopTarget::InstanceTitleBar(instance),
        ];
        if let Some(view) = self
            .aggregates
            .instances
            .get(&instance)
            .and_then(|presenter| presenter.primary_view_id())
        {
            targets.push(DesktopTarget::View(view));
        }
        targets
    }
}

/// Adds a project and its header and matrix under the hosting target.
fn project_topology(project: ProjectId, under: DesktopTarget) -> Changes {
    let mut changes = Changes::Empty;
    changes <<= TopologyChange::Add {
        what: DesktopTarget::Project(project),
        under,
        after: None,
    };
    changes <<= TopologyChange::AddNested {
        what: [
            DesktopTarget::ProjectHeader(project),
            DesktopTarget::ProjectMatrix(project),
        ]
        .into(),
        under: DesktopTarget::Project(project),
    };
    changes
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use massive_animation::{AnimationCoordinator, MovementRuntime};
    use massive_geometry::{Rect, RectPx, SizePx};
    use massive_layout::LayoutTopology;
    use massive_renderer::{FontManager, ShapingEngineKind};
    use massive_scene::{AnyCollector, SceneChange};

    use super::*;
    use crate::desktop_environment::DesktopEnvironment;
    use crate::desktop_system::TransactionEffectsMode;
    use crate::desktop_system::change::DesktopSystemEffect;
    use crate::desktop_system::change::Zoom;
    use crate::instance_manager::InstanceManager;
    use crate::instance_presenter::{InstanceKind, InstanceTitleBarMetrics};
    use crate::projects::persistence::parse_configuration;
    use crate::window_state::WindowState;
    use massive_applications::task_context::{self, TaskContext};
    use massive_applications::{InstanceEnvironment, InstanceSubmission};
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    /// Tests run at the scale factor of a standard display.
    const TEST_SCALE_FACTOR: f64 = 1.0;

    const CONFIG: &str = r#"{
        "startup": "/shell",
        "slots": [
            { "at": [0, 0], "launcher": { "name": "shell", "mode": "visor", "params": { "command": "echo" } } }
        ]
    }"#;

    /// The startup launcher nested in a project, so it sits inside a project slot.
    const NESTED_PROJECT_CONFIG: &str = r#"{
        "startup": "/labs/shell",
        "slots": [
            { "at": [0, 0], "project": { "name": "labs", "slots": [
                { "at": [0, 0], "launcher": { "name": "shell", "mode": "visor" } }
            ] } }
        ]
    }"#;

    const NESTED_SIBLINGS_CONFIG: &str = r#"{
        "startup": "/labs-a/shell-a",
        "slots": [
            { "at": [0, 0], "project": { "name": "labs-a", "slots": [
                { "at": [0, 0], "launcher": { "name": "shell-a", "mode": "visor" } }
            ] } },
            { "at": [1, 0], "project": { "name": "labs-b", "slots": [
                { "at": [0, 0], "launcher": { "name": "shell-b", "mode": "visor" } }
            ] } }
        ]
    }"#;

    /// The nested siblings booted, with one instance started per sibling and `labs-a`'s focused.
    struct NestedSiblings {
        system: DesktopSystem,
        instance_manager: InstanceManager,
        project_b: ProjectId,
        instance_a: InstanceId,
        instance_b: InstanceId,
    }

    fn nested_siblings() -> Result<NestedSiblings> {
        let (mut system, receiver) = system_from(NESTED_SIBLINGS_CONFIG);
        let mut instance_manager = instance_manager(&receiver);
        for command in crate::projects::to_commands(&system.aggregates.configuration) {
            let changes = system.plan(DesktopCommand::Project(command))?;
            system.transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Setup,
            )?;
        }

        let launcher_a = system
            .aggregates
            .configuration
            .boot_launcher()
            .expect("the test configuration has a startup launcher");
        let project_a = system
            .aggregates
            .hierarchy
            .project_of_target(&DesktopTarget::Launcher(launcher_a));
        let project_b = system
            .aggregates
            .configuration
            .slots_ordered(ProjectId::ROOT)
            .find_map(|(_, content)| match content {
                SlotContent::Project(project) if project != project_a => Some(project),
                _ => None,
            })
            .expect("the root has a sibling project");
        let launcher_b = system
            .aggregates
            .configuration
            .slots_ordered(project_b)
            .find_map(|(_, content)| match content {
                SlotContent::Launcher(launcher) => Some(launcher),
                SlotContent::Project(_) => None,
            })
            .expect("the sibling project has a launcher");

        let instance_a = start_instance(&mut system, &mut instance_manager, launcher_a, 0)?;
        let instance_b = start_instance(&mut system, &mut instance_manager, launcher_b, 0)?;
        system.transact(
            set_focus(
                Some(DesktopTarget::Instance(instance_a)),
                KeyboardFocusReason::InputTransition,
            ),
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;

        Ok(NestedSiblings {
            system,
            instance_manager,
            project_b,
            instance_a,
            instance_b,
        })
    }

    fn run(
        system: &mut DesktopSystem,
        instance_manager: &mut InstanceManager,
        command: DesktopCommand,
    ) -> Result<()> {
        let changes = system.plan(command)?;
        system.transact(changes, instance_manager, TransactionEffectsMode::Setup)?;
        Ok(())
    }

    #[tokio::test]
    async fn unified_requests_assign_under_a_path_and_remove_both_content_kinds() -> Result<()> {
        task_context::with_context(task_context(), async {
            let frame = massive_applications::begin_frame();
            let NestedSiblings {
                mut system,
                mut instance_manager,
                project_b,
                instance_a,
                instance_b,
                ..
            } = nested_siblings()?;
            let placement = MatrixPlacement { column: 1, row: 1 };
            for target in [ConfigurationTarget::Project, ConfigurationTarget::Launcher] {
                let output = system.apply_configuration_request(
                    instance_a,
                    ConfigurationRequest::Assign {
                        target,
                        name: "added".into(),
                        column: placement.column,
                        row: placement.row,
                        under: Some("/labs-b".into()),
                        shift: SlotShift::Keep,
                    },
                )?;
                system.transact(
                    output.changes,
                    &mut instance_manager,
                    TransactionEffectsMode::Setup,
                )?;
                let content = system
                    .aggregates
                    .configuration
                    .content_at(project_b, placement)
                    .expect("the request assigns content under the resolved project");
                assert!(matches!(
                    (target, content),
                    (ConfigurationTarget::Project, SlotContent::Project(_))
                        | (ConfigurationTarget::Launcher, SlotContent::Launcher(_))
                ));
                assert!(system.aggregates.hierarchy.exists(&content.target()));
                if let SlotContent::Project(project) = content {
                    assert_eq!(
                        system.aggregates.hierarchy.get_nested(&content.target()),
                        &[
                            DesktopTarget::ProjectHeader(project),
                            DesktopTarget::ProjectMatrix(project)
                        ]
                    );
                }
                let output = system.apply_configuration_request(
                    instance_b,
                    ConfigurationRequest::Remove {
                        target,
                        name: Some("added".into()),
                    },
                )?;
                system.transact(
                    output.changes,
                    &mut instance_manager,
                    TransactionEffectsMode::Setup,
                )?;
                assert_eq!(
                    system
                        .aggregates
                        .configuration
                        .content_at(project_b, placement),
                    None
                );
                assert!(!system.aggregates.hierarchy.exists(&content.target()));
            }
            drop(frame);
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn navigation_escapes_to_the_parent_and_restores_focus_on_return() -> Result<()> {
        task_context::with_context(task_context(), async {
            let frame = massive_applications::begin_frame();
            let NestedSiblings {
                mut system,
                mut instance_manager,
                instance_a,
                instance_b,
                ..
            } = nested_siblings()?;

            for _ in 0..2 {
                run(
                    &mut system,
                    &mut instance_manager,
                    DesktopCommand::Zoom(Zoom::Out),
                )?;
            }
            assert_eq!(system.focused_zoom_level(), Some((ZoomLevel::Row, 1)));

            // `labs-a` holds a single slot, so there is nothing to its right inside it: the
            // navigation escapes to the sibling project, which was visited and so is re-entered
            // at its focus leaf. Navigating back restores the original focus.
            let navigate =
                |system: &mut DesktopSystem, manager: &mut InstanceManager, direction| {
                    run(system, manager, DesktopCommand::Navigate(direction))
                };
            navigate(
                &mut system,
                &mut instance_manager,
                crate::desktop_system::Direction::Right,
            )?;
            assert_eq!(
                system.event_router.keyboard_focus(),
                Some(&DesktopTarget::Instance(instance_b))
            );
            assert_eq!(
                system.focused_zoom_level().map(|(level, _)| level),
                Some(ZoomLevel::Row)
            );

            navigate(
                &mut system,
                &mut instance_manager,
                crate::desktop_system::Direction::Left,
            )?;
            assert_eq!(
                system.event_router.keyboard_focus(),
                Some(&DesktopTarget::Instance(instance_a))
            );

            drop(frame.submission::<SceneChange>());
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn zooming_out_in_the_root_keeps_focus_on_the_root_slot() -> Result<()> {
        task_context::with_context(task_context(), async {
            let frame = massive_applications::begin_frame();
            let (mut system, receiver) = system();
            let mut instance_manager = instance_manager(&receiver);
            for command in crate::projects::to_commands(&system.aggregates.configuration) {
                run(
                    &mut system,
                    &mut instance_manager,
                    DesktopCommand::Project(command),
                )?;
            }
            let launcher = system
                .aggregates
                .configuration
                .boot_launcher()
                .expect("the test configuration has a startup launcher");
            let instance = start_instance(&mut system, &mut instance_manager, launcher, 0)?;

            for expected in [ZoomLevel::Slot, ZoomLevel::Row, ZoomLevel::Project] {
                run(
                    &mut system,
                    &mut instance_manager,
                    DesktopCommand::Zoom(Zoom::Out),
                )?;
                assert_eq!(system.focused_zoom_level(), Some((expected, 0)));
                assert_eq!(
                    system.event_router.keyboard_focus(),
                    Some(&DesktopTarget::Instance(instance))
                );
            }
            assert!(system.plan(DesktopCommand::Zoom(Zoom::Out))?.is_empty());

            drop(frame.submission::<SceneChange>());
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn a_parentless_assign_slot_creates_the_root_under_desktop() -> Result<()> {
        task_context::with_context(task_context(), async {
            let (mut system, receiver) = system();
            let mut instance_manager = instance_manager(&receiver);

            let changes = system.plan(DesktopCommand::Project(ProjectCommand::AssignSlot {
                parent: None,
                placement: MatrixPlacement { column: 0, row: 0 },
                content: SlotAssignment::Project {
                    id: ProjectId::ROOT,
                    name: crate::projects::ROOT_PROJECT_NAME.into(),
                },
                shift: SlotShift::default(),
            }))?;
            system.transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Setup,
            )?;

            let hierarchy = &system.aggregates.hierarchy;
            assert!(
                hierarchy
                    .parent_of(&DesktopTarget::Project(ProjectId::ROOT))
                    .is_some(),
                "the root project hangs under the implicit Desktop root"
            );
            assert!(hierarchy.exists(&DesktopTarget::Desktop));
            assert!(system.aggregates.projects.get(&ProjectId::ROOT).is_some());

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn startup_setup_snaps_camera_to_the_focused_instance() -> Result<()> {
        task_context::with_context(task_context(), async {
            let (mut system, receiver) = system();
            let mut instance_manager = instance_manager(&receiver);
            let launcher = system
                .aggregates
                .configuration
                .boot_launcher()
                .expect("the test configuration has a startup launcher");
            let frame = massive_applications::begin_frame();
            for command in crate::projects::to_commands(&system.aggregates.configuration) {
                let changes = system.plan(DesktopCommand::Project(command))?;
                system.transact(
                    changes,
                    &mut instance_manager,
                    TransactionEffectsMode::Setup,
                )?;
            }

            let instance = uuid::Uuid::new_v4().into();
            let start = system.plan(DesktopCommand::StartInstance {
                launcher,
                instance,
                root: Some(InstanceRoot::new()),
                parameters: Default::default(),
                kind: InstanceKind::Primary,
            })?;
            let mut submission_changes = massive_util::ChangeSet::default();
            submission_changes.push(InstanceChange::CreateView(
                massive_applications::ViewCreationInfo {
                    id: uuid::Uuid::new_v4().into(),
                    role: ViewRole::Primary,
                    extents: massive_geometry::BoxPx::new(
                        massive_geometry::PointPx::new(0, 0),
                        massive_geometry::PointPx::new(800, 600),
                    ),
                },
            ));
            let initial_submission = InstanceSubmission::new(
                submission_changes,
                massive_renderer::RenderPacing::default(),
            );
            let mut changes: Changes = start;
            changes <<= DesktopChange::IntegrateInstanceSubmission(instance, initial_submission);
            system.transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Setup,
            )?;

            assert_eq!(
                system.event_router.keyboard_focus(),
                Some(&DesktopTarget::View(
                    system.aggregates.instances[&instance]
                        .primary_view_id()
                        .expect("the initial submission created a primary view")
                ))
            );
            assert_eq!(system.zoom_level, ZoomLevel::Focus);
            let focused = system.event_router.keyboard_focus().unwrap();
            let expected = system.resolve_camera_for_target(focused, system.zoom_level);
            assert_eq!(
                *system.camera(),
                expected,
                "setup should snap to the focused instance camera without an initial transition"
            );

            drop(frame.submission::<SceneChange>());
            Ok(())
        })
        .await
    }

    /// Regression (ADR 0014): toggling Full Screen Mode must rescale the focused instance's
    /// view in the same transaction, not only after focus moved away and back.
    /// The fullscreen scale lives on the instance's view placement, so the
    /// toggle must invalidate the whole launcher subtree (views included), not
    /// just the launcher's own measurement.
    #[tokio::test]
    async fn toggling_full_screen_mode_rescales_the_focused_view_in_place() -> Result<()> {
        task_context::with_context(task_context(), async {
            let (mut system, receiver) = system();
            let mut instance_manager = instance_manager(&receiver);
            let frame = massive_applications::begin_frame();
            for command in crate::projects::to_commands(&system.aggregates.configuration) {
                // Tests at panel size: the system's constructor already
                // committed the 800×600 window state.
                let changes = system.plan(DesktopCommand::Project(command))?;
                system.transact(changes, &mut instance_manager, TransactionEffectsMode::Setup)?;
            }

            // A native fullscreen 1000×800 window on the 800×600 panel: the fullscreen toggle
            // frames content at the window, so the tests need the larger state
            // committed through its change.
            system.transact(
                DesktopChange::WindowResized(WindowState::new(SizePx::new(1000, 800), true)),
                &mut instance_manager,
                TransactionEffectsMode::Setup,
            )?;

            let launcher = system
                .aggregates
                .configuration
                .boot_launcher()
                .expect("the test configuration has a startup launcher");
            let instance = uuid::Uuid::new_v4().into();
            let start = system.plan(DesktopCommand::StartInstance {
                launcher,
                instance,
                root: Some(InstanceRoot::new()),
                parameters: Default::default(),
                kind: InstanceKind::Primary,
            })?;
            let mut submission_changes = massive_util::ChangeSet::default();
            submission_changes.push(InstanceChange::CreateView(
                massive_applications::ViewCreationInfo {
                    id: uuid::Uuid::new_v4().into(),
                    role: ViewRole::Primary,
                    extents: massive_geometry::BoxPx::new(
                        massive_geometry::PointPx::new(0, 0),
                        massive_geometry::PointPx::new(800, 600),
                    ),
                },
            ));
            let initial_submission = InstanceSubmission::new(
                submission_changes,
                massive_renderer::RenderPacing::default(),
            );
            let mut changes: Changes = start;
            changes <<= DesktopChange::IntegrateInstanceSubmission(instance, initial_submission);
            system.transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Setup
)?;

            let view = system.aggregates.instances[&instance]
                .primary_view_id()
                .expect("the initial submission created a primary view");
            let scale_before = system
                .placement(&DesktopTarget::View(view))
                .transform
                .scale;

            // Toggle through the live command path, like the desktop loop does.
            let changes = system.plan(DesktopCommand::ToggleFullScreen)?;
            system.transact(
                changes,
                &mut instance_manager,
                Option::<TransactionEffectsMode>::None
)?;

            let scale_after = system
                .placement(&DesktopTarget::View(view))
                .transform
                .scale;

            assert!(
                system.aggregates.configuration[launcher].full_screen_mode
                    == crate::projects::FullScreenMode::FullScreen,
                "the toggle flipped the launcher's Full Screen Mode"
            );
            assert!(
                (scale_after - scale_before).abs() > 0.001,
                "the focused view must rescale in the toggle transaction: before {scale_before:?}, after {scale_after:?}"
            );

            // Toggle back off: the visor layout must collapse back onto panel
            // measurements in the same transaction — not only after the next
            // focus change or arc rotation (regression: stale view measurement).
            let instance_b = uuid::Uuid::new_v4().into();
            let start_b = system.plan(DesktopCommand::StartInstance {
                launcher,
                instance: instance_b,
                root: Some(InstanceRoot::new()),
                parameters: Default::default(),
                kind: InstanceKind::Primary,
            })?;
            let mut submission_b = massive_util::ChangeSet::default();
            submission_b.push(InstanceChange::CreateView(
                massive_applications::ViewCreationInfo {
                    id: uuid::Uuid::new_v4().into(),
                    role: ViewRole::Primary,
                    extents: massive_geometry::BoxPx::new(
                        massive_geometry::PointPx::new(0, 0),
                        massive_geometry::PointPx::new(800, 600),
                    ),
                },
            ));
            let mut changes_b: Changes = start_b;
            changes_b <<= DesktopChange::IntegrateInstanceSubmission(
                instance_b,
                InstanceSubmission::new(
                    submission_b,
                    massive_renderer::RenderPacing::default(),
                ),
            );
            system.transact(
                changes_b,
                &mut instance_manager,
                Option::<TransactionEffectsMode>::None,
            )?;

            let view_b = system.aggregates.instances[&instance_b]
                .primary_view_id()
                .expect("the second instance created a primary view");
            let view_b_size_fullscreen = system
                .placement(&DesktopTarget::View(view_b))
                .rect
                .size;

            let changes = system.plan(DesktopCommand::ToggleFullScreen)?;
            system.transact(
                changes,
                &mut instance_manager,
                Option::<TransactionEffectsMode>::None
)?;

            let view_b_size_after = system
                .placement(&DesktopTarget::View(view_b))
                .rect
                .size;
            assert!(
                system.aggregates.configuration[launcher].full_screen_mode
                    == crate::projects::FullScreenMode::Regular,
                "the second toggle restored the launcher's Regular mode"
            );
            assert!(
                (view_b_size_after[0] as f64 - 800.0).abs() < 0.01
                    && view_b_size_fullscreen[0] as f64 >= 1000.0,
                "the visor view must be measured back to its panel size in the toggle-off transaction: fullscreen {view_b_size_fullscreen:?}, after {view_b_size_after:?}"
            );

            drop(frame.submission::<SceneChange>());
            Ok(())
        })
        .await
    }

    /// The camera and the hover read a target's absolute placement, so the scene must
    /// resolve that target to the same transform — a project nested in a parent slot
    /// included.
    #[tokio::test]
    async fn nested_project_launcher_scene_transform_matches_its_placement() -> Result<()> {
        task_context::with_context(task_context(), async {
            let (mut system, receiver) = system_from(NESTED_PROJECT_CONFIG);
            let mut instance_manager = instance_manager(&receiver);
            let frame = massive_applications::begin_frame();
            for command in crate::projects::to_commands(&system.aggregates.configuration) {
                let changes = system.plan(DesktopCommand::Project(command))?;
                system.transact(
                    changes,
                    &mut instance_manager,
                    TransactionEffectsMode::Setup
)?;
            }

            let launcher = system
                .aggregates
                .configuration
                .boot_launcher()
                .expect("the test configuration has a startup launcher");
            let location = system.aggregates.launchers[&launcher].location();
            let resolved = massive_scene::TransformResolver::default()
                .resolve(&location.to_ref())
                .transform;

            let placement = system.placement(&DesktopTarget::Launcher(launcher));
            let rect_px: RectPx = placement.rect.into();
            let local_center = Rect::from(rect_px).size().to_rect().center();
            let expected = placement.transform.to_origin_space(local_center);

            assert_eq!(resolved.scale, expected.scale);
            let offset = resolved.translate - expected.translate;
            assert!(
                offset.x.abs() < 0.001 && offset.y.abs() < 0.001 && offset.z.abs() < 0.001,
                "the scene draws the launcher at {resolved:?}, but the model places it at {expected:?}"
            );

            drop(frame.submission::<SceneChange>());
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn a_second_parentless_assign_slot_is_rejected() -> Result<()> {
        task_context::with_context(task_context(), async {
            let (mut system, receiver) = system();
            let mut instance_manager = instance_manager(&receiver);

            let changes = system.plan(DesktopCommand::Project(ProjectCommand::AssignSlot {
                parent: None,
                placement: MatrixPlacement { column: 0, row: 0 },
                content: SlotAssignment::Project {
                    id: ProjectId::ROOT,
                    name: crate::projects::ROOT_PROJECT_NAME.into(),
                },
                shift: SlotShift::default(),
            }))?;
            system.transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Setup,
            )?;

            for id in [ProjectId::ROOT, ProjectId::new()] {
                assert!(
                    system
                        .plan(DesktopCommand::Project(ProjectCommand::AssignSlot {
                            parent: None,
                            placement: MatrixPlacement { column: 0, row: 0 },
                            content: SlotAssignment::Project {
                                id,
                                name: "again".into(),
                            },
                            shift: SlotShift::default(),
                        }))
                        .is_err(),
                    "a second parentless AssignSlot must be rejected"
                );
            }

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn the_boot_replay_builds_the_tree_through_plan_and_transact() -> Result<()> {
        task_context::with_context(task_context(), async {
            let (mut system, receiver) = system();
            let mut instance_manager = instance_manager(&receiver);

            let aggregate = parse_configuration(
                Path::new("/config/desktop.json"),
                r#"{ "slots": [ { "at": [0, 0], "project": { "name": "work", "slots": [
                    { "at": [0, 0], "launcher": { "name": "shell", "mode": "visor" } }
                ] } } ] }"#,
            )?;
            for command in crate::projects::to_commands(&aggregate) {
                let changes = system.plan(DesktopCommand::Project(command))?;
                system.transact(
                    changes,
                    &mut instance_manager,
                    TransactionEffectsMode::Setup,
                )?;
            }

            let hierarchy = &system.aggregates.hierarchy;
            assert!(hierarchy.exists(&DesktopTarget::Desktop));
            let root = DesktopTarget::Project(ProjectId::ROOT);
            assert!(hierarchy.exists(&root));
            assert!(hierarchy.exists(&DesktopTarget::ProjectMatrix(ProjectId::ROOT)));
            let nested_slot = hierarchy
                .get_nested(&DesktopTarget::ProjectMatrix(ProjectId::ROOT))
                .first()
                .cloned()
                .expect("the boot replay assigned the parsed project slot");
            assert!(matches!(nested_slot, DesktopTarget::Project(_)));

            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn fullscreen_request_enters_native_fullscreen_before_toggling_instance_mode()
    -> Result<()> {
        task_context::with_context(task_context(), async {
            let frame = massive_applications::begin_frame();
            let (mut system, mut instance_manager, launcher) = fullscreen_system(CONFIG)?;
            let instance = start_instance(&mut system, &mut instance_manager, launcher, 0)?;
            assert_eq!(system.zoom_level, ZoomLevel::Focus);
            assert_eq!(system.focused_path().instance(), Some(instance));

            let changes = system.plan(DesktopCommand::ToggleFullScreen)?;
            assert!(matches!(
                changes,
                Changes::One(DesktopChange::ToggleWindowFullScreen)
            ), "a windowed native window must enter fullscreen regardless of instance focus: {changes:?}");
            let output = system.transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Normal,
            )?;
            assert!(matches!(
                output.effects.as_slice(),
                [DesktopSystemEffect::ToggleWindowFullScreen]
            ));

            system.transact(
                DesktopChange::WindowResized(WindowState::new(SizePx::new(1000, 800), true)),
                &mut instance_manager,
                TransactionEffectsMode::Setup,
            )?;
            let changes = system.plan(DesktopCommand::ToggleFullScreen)?;
            assert!(matches!(
                changes,
                Changes::One(DesktopChange::ToggleFullScreenMode(
                    ToggleFullScreenModeTarget::Launcher(target)
                )) if target == launcher
            ));
            let output = system.transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Normal,
            )?;
            assert!(matches!(
                output.effects.as_slice(),
                [crate::desktop_system::change::DesktopSystemEffect::PersistConfiguration]
            ));

            drop(frame.submission::<SceneChange>());
            Ok(())
        })
        .await
    }

    #[tokio::test]
    async fn native_fullscreen_resize_keeps_the_presented_camera_on_the_focused_instance()
    -> Result<()> {
        task_context::with_context(task_context(), async {
            let frame = massive_applications::begin_frame();
            let (mut system, mut instance_manager, launcher) = fullscreen_system(CONFIG)?;
            let instance = start_instance(&mut system, &mut instance_manager, launcher, 0)?;
            deliver_view(&mut system, &mut instance_manager, instance)?;
            let focused = system.event_router.keyboard_focus().cloned();

            for window_state in [
                WindowState::new(SizePx::new(1000, 864), false),
                WindowState::new(SizePx::new(2560, 1440), true),
            ] {
                system.transact(
                    DesktopChange::WindowResized(window_state),
                    &mut instance_manager,
                    Option::<TransactionEffectsMode>::None,
                )?;
                assert_eq!(system.event_router.keyboard_focus(), focused.as_ref());
                assert_eq!(system.zoom_level, ZoomLevel::Focus);
                let desired = system
                    .resolve_desired_camera()
                    .expect("the instance is focused");
                assert_eq!(
                    *system.camera(),
                    desired,
                    "native resize must present the focused camera without an intermediate zoom"
                );
            }

            drop(frame.submission::<SceneChange>());
            Ok(())
        })
        .await
    }

    fn task_context() -> TaskContext {
        TaskContext::new(
            AnyCollector::for_type::<SceneChange>(),
            AnimationCoordinator::new(),
            MovementRuntime::default(),
            FontManager::system(ShapingEngineKind::available()[0])
                .expect("system fonts are available")
                .new_shaping_context(),
        )
    }

    fn system() -> (
        DesktopSystem,
        UnboundedReceiver<(InstanceId, InstanceSubmission)>,
    ) {
        system_from(CONFIG)
    }

    fn system_from(
        config: &str,
    ) -> (
        DesktopSystem,
        UnboundedReceiver<(InstanceId, InstanceSubmission)>,
    ) {
        let (_sender, receiver) = unbounded_channel();
        let environment = DesktopEnvironment {
            primary_application: "terminal".into(),
            applications: crate::application_registry::ApplicationRegistry::new(Vec::new()),
            projects_dir: None,
        };
        let path = Path::new("/config/desktop.json");
        let aggregate = parse_configuration(path, config).unwrap();
        let system = DesktopSystem::new(
            environment,
            SizePx::new(800, 600),
            InstanceTitleBarMetrics::from_scale_factor(TEST_SCALE_FACTOR),
            aggregate,
        )
        .unwrap();
        (system, receiver)
    }

    fn instance_manager(
        _receiver: &UnboundedReceiver<(InstanceId, InstanceSubmission)>,
    ) -> InstanceManager {
        let (sender, _receiver) = unbounded_channel();
        InstanceManager::new(InstanceEnvironment::new(sender, 1.0))
    }

    /// A system booted from `config` in a 1000×800 window, its startup launcher
    /// in Full Screen Mode. On the 800×600 panel, fullscreen framing pins the
    /// window size and scales content by 0.75.
    fn fullscreen_system(
        config: &str,
    ) -> Result<(DesktopSystem, InstanceManager, LaunchProfileId)> {
        let (mut system, receiver) = system_from(config);
        let mut instance_manager = instance_manager(&receiver);
        for command in crate::projects::to_commands(&system.aggregates.configuration) {
            let changes = system.plan(DesktopCommand::Project(command))?;
            system.transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Setup,
            )?;
        }
        system.transact(
            DesktopChange::WindowResized(WindowState::new(SizePx::new(1000, 800), false)),
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;

        let launcher = system
            .aggregates
            .configuration
            .boot_launcher()
            .expect("the test configuration has a startup launcher");
        let launcher_config = system
            .aggregates
            .configuration
            .launcher_mut(launcher)
            .unwrap();
        launcher_config.full_screen_mode = launcher_config.full_screen_mode.toggled();
        Ok((system, instance_manager, launcher))
    }

    /// The live StartInstance commit: a focused primary instance whose view has not
    /// arrived yet.
    fn start_instance(
        system: &mut DesktopSystem,
        instance_manager: &mut InstanceManager,
        launcher: LaunchProfileId,
        index: usize,
    ) -> Result<InstanceId> {
        let instance = uuid::Uuid::new_v4().into();
        let mut changes: Changes = [
            DesktopChange::PresentInstance(InstancePresentation {
                launcher,
                initial_center_translation: None,
                instance,
                root: InstanceRoot::new(),
                parameters: Default::default(),
                kind: InstanceKind::Primary,
            }),
            DesktopChange::Topology(TopologyChange::Insert {
                what: instance.into(),
                at_index: index,
                under: launcher.into(),
            }),
            DesktopChange::Topology(TopologyChange::Add {
                what: DesktopTarget::InstanceTitleBar(instance),
                under: DesktopTarget::Instance(instance),
                after: None,
            }),
        ]
        .into();
        changes += set_focus(
            Some(DesktopTarget::Instance(instance)),
            KeyboardFocusReason::PresentInstance,
        );
        system.transact(changes, instance_manager, TransactionEffectsMode::Setup)?;
        Ok(instance)
    }

    /// The instance's first submission, which creates its primary view.
    fn deliver_view(
        system: &mut DesktopSystem,
        instance_manager: &mut InstanceManager,
        instance: InstanceId,
    ) -> Result<()> {
        let mut submission = massive_util::ChangeSet::default();
        submission.push(InstanceChange::CreateView(
            massive_applications::ViewCreationInfo {
                id: uuid::Uuid::new_v4().into(),
                role: ViewRole::Primary,
                extents: massive_geometry::BoxPx::new(
                    massive_geometry::PointPx::new(0, 0),
                    massive_geometry::PointPx::new(800, 600),
                ),
            },
        ));
        system.transact(
            DesktopChange::IntegrateInstanceSubmission(
                instance,
                InstanceSubmission::new(submission, massive_renderer::RenderPacing::default()),
            ),
            instance_manager,
            TransactionEffectsMode::Setup,
        )?;
        Ok(())
    }

    fn camera_distance(system: &DesktopSystem) -> f64 {
        system
            .resolve_desired_camera()
            .expect("a focused instance resolves a camera")
            .distance
    }

    /// A primary instance created in a Full Screen launcher spawns at window
    /// resolution and frames fullscreen from its view-less first commit;
    /// framing it at the panel distance dollies the camera out and back in when
    /// the view arrives (the `Cmd+T` zoom-out bounce).
    #[tokio::test]
    async fn a_new_base_instance_in_a_fullscreen_launcher_frames_fullscreen_from_its_first_commit()
    -> Result<()> {
        task_context::with_context(task_context(), async {
            let frame = massive_applications::begin_frame();
            let (mut system, mut instance_manager, launcher) = fullscreen_system(CONFIG)?;

            // A real spawn: root None, so plan emits SpawnInstance with the
            // seed parameters whose `size_px` this test pins.
            let start = system.plan(DesktopCommand::StartInstance {
                launcher,
                instance: uuid::Uuid::new_v4().into(),
                root: None,
                parameters: Default::default(),
                kind: InstanceKind::Primary,
            })?;
            let seed = start
                .iter()
                .find_map(|change| match change {
                    DesktopChange::SpawnInstance { parameters, .. } => {
                        parameters.get("size_px").cloned()
                    }
                    _ => None,
                })
                .expect("SpawnInstance seeds the application canvas size");
            assert_eq!(
                seed,
                serde_json::json!([1000, 800 - system.title_bar.height]),
                "a fullscreen launcher's spawned instance must start at window resolution below the title bar, not panel"
            );

            // The plan's SpawnInstance would hit the (empty) test application
            // registry, so the instance is presented directly.
            let instance = start_instance(&mut system, &mut instance_manager, launcher, 0)?;

            let panel_distance = massive_geometry::PixelCamera::pixel_perfect_distance(
                massive_geometry::PixelCamera::DEFAULT_FOVY,
            );
            // The window-resolution stack (title bar and view) fits the instance extent: panel and bar.
            let fullscreen_distance = panel_distance
                * (800.0_f64 / 1000.0).min((600.0 + system.title_bar.height as f64) / 800.0);
            let view_less_distance = camera_distance(&system);
            assert!(
                (view_less_distance - fullscreen_distance).abs() < 1e-6,
                "a view-less fullscreen instance must frame at the fullscreen distance: expected {fullscreen_distance}, got {view_less_distance}"
            );

            deliver_view(&mut system, &mut instance_manager, instance)?;
            let view_distance = camera_distance(&system);
            assert!(
                (view_distance - fullscreen_distance).abs() < 1e-6,
                "the fullscreen instance keeps the fullscreen distance once its view arrives: expected {fullscreen_distance}, got {view_distance}"
            );

            drop(frame.submission::<SceneChange>());
            Ok(())
        })
        .await
    }

    /// Regression: `Cmd+T` on a fullscreen launcher must keep camera framing stable
    /// while the new instance waits for its first view and once that view arrives.
    #[tokio::test]
    async fn cmd_t_on_a_fullscreen_instance_keeps_camera_stable() -> Result<()> {
        cmd_t_keeps_camera_stable(CONFIG).await
    }

    /// The same while the launcher sits in a nested project's scaled slot.
    #[tokio::test]
    async fn cmd_t_in_a_nested_slot_keeps_camera_stable() -> Result<()> {
        cmd_t_keeps_camera_stable(NESTED_PROJECT_CONFIG).await
    }

    async fn cmd_t_keeps_camera_stable(config: &str) -> Result<()> {
        task_context::with_context(task_context(), async {
            let frame = massive_applications::begin_frame();
            let (mut system, mut instance_manager, launcher) = fullscreen_system(config)?;

            // Instance A's framing is the baseline: in a nested slot it carries
            // the slot scale.
            let instance_a = start_instance(&mut system, &mut instance_manager, launcher, 0)?;
            deliver_view(&mut system, &mut instance_manager, instance_a)?;
            let distance = camera_distance(&system);

            // `Cmd+T`: instance B is focused one commit before its view exists.
            let instance_b = start_instance(&mut system, &mut instance_manager, launcher, 1)?;
            let view_less_distance = camera_distance(&system);
            assert!(
                (view_less_distance - distance).abs() < 1e-6,
                "the camera must not zoom out while the new instance is view-less: expected {distance}, got {view_less_distance}"
            );

            deliver_view(&mut system, &mut instance_manager, instance_b)?;
            let view_distance = camera_distance(&system);
            assert!(
                (view_distance - distance).abs() < 1e-6,
                "the camera must not move once the view arrives: expected {distance}, got {view_distance}"
            );

            drop(frame.submission::<SceneChange>());
            Ok(())
        })
        .await
    }
}
