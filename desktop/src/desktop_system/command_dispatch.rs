use anyhow::{Context, Result};
use log::{debug, warn};
use serde_json::json;

use massive_applications::{
    ConfigurationRequest, CreationMode, InstanceChange, InstanceId, InstanceSubmission, ViewChange,
    ViewEvent, ViewRole,
};
use massive_shell::{Frame, Scene};

use super::change::Zoom;
use super::change::set_focus;
use super::change::{Changes, DesktopChange, ProjectChange, TopologyChange};
use super::navigation::focus_depth_from_target;
use super::{
    ChangeSurface, DesktopCommand, DesktopSystem, DesktopTarget, FocusDepth, KeyboardFocusReason,
    ProjectCommand,
};
use crate::desktop_system::change_surface::TargetSet;
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::instance_presenter::InstanceRoot;
use crate::projects::{
    LaunchProfile, LaunchProfileId, LauncherMode, LauncherPresenter, MatrixPlacement, ProjectId,
    ProjectPresenter, ProjectProperties,
};
use crate::{MatrixPositions, RemoveSlotShiftingPolicy};

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

    pub fn focus_changed(&mut self, target: DesktopTarget) {
        self.surface.size_invalid += target;
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
    pub fn plan(&self, command: DesktopCommand, scene: &Scene) -> Result<Changes> {
        match command {
            DesktopCommand::Project(project_command) => return self.plan_project(project_command),
            DesktopCommand::StartInstance {
                launcher,
                instance,
                root,
                parameters,
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
                    None => (InstanceRoot::new(scene), true),
                };

                let mut changes: Changes = if spawn {
                    vec![DesktopChange::SpawnInstance {
                        instance,
                        root: root.clone(),
                        parameters: parameters.clone(),
                    }]
                } else {
                    Vec::new()
                }
                .into();

                changes += [
                    DesktopChange::PresentInstance {
                        launcher,
                        initial_center_translation: originating_details
                            .and_then(|od| od.initial_center_translation),
                        instance,
                        root,
                        parameters,
                    },
                    DesktopChange::Topology(TopologyChange::Insert {
                        what: instance.into(),
                        at_index: insertion_pos,
                        under: launcher.into(),
                    }),
                ];
                changes += set_focus(
                    Some(DesktopTarget::Instance(instance)),
                    KeyboardFocusReason::PresentInstance,
                );
                changes <<= DesktopChange::CommitFocusDepth(FocusDepth::default());

                return Ok(changes);
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
                changes <<= DesktopChange::CommitFocusDepth(FocusDepth::default());

                return Ok(changes);
            }
            DesktopCommand::Navigate(direction) => return self.plan_navigate(direction),
            DesktopCommand::Zoom(Zoom::In) => {
                if let Some(focus_depth) = self.focus_depth.zoom_in() {
                    return Ok(DesktopChange::CommitFocusDepth(focus_depth).into());
                }
            }
            DesktopCommand::Zoom(Zoom::Out) => {
                if let Some(focus_depth) = self.focus_depth.zoom_out() {
                    return Ok(DesktopChange::CommitFocusDepth(focus_depth).into());
                }
            }
            DesktopCommand::Zoom(Zoom::DefaultForFocused) => {
                if let Some(keyboard_focus) = self.event_router.keyboard_focus() {
                    let current_level = self.focus_depth;
                    let focus_level = focus_depth_from_target(keyboard_focus);

                    if current_level != focus_level {
                        return Ok(DesktopChange::CommitFocusDepth(focus_level).into());
                    }
                }
            }
        }

        Ok([].into())
    }

    fn plan_project(&self, command: ProjectCommand) -> Result<Changes> {
        let mut changes = Changes::Empty;
        match command {
            ProjectCommand::AddProject {
                id,
                properties,
                after,
            } => {
                let parent_target = DesktopTarget::Desktop;
                let project_target = DesktopTarget::Project(id);

                changes <<= TopologyChange::Add {
                    what: project_target.clone(),
                    under: parent_target,
                    after: after.map(DesktopTarget::Project),
                };

                changes <<= TopologyChange::AddNested {
                    what: [
                        DesktopTarget::ProjectHeader(id),
                        DesktopTarget::ProjectMatrix(id),
                    ]
                    .into(),
                    under: project_target,
                };
                changes <<= ProjectChange::AddProject { id, properties };
            }
            ProjectCommand::RemoveProject(project_id) => {
                changes += self.plan_project_removal_focus(project_id);
                changes += self.plan_remove_project(project_id);
            }
            ProjectCommand::AddLauncher {
                project,
                id: launch_profile_id,
                profile,
                placement,
            } => {
                let mut launchers = self.aggregates.hierarchy.matrix_launchers(project);
                if let Some(launcher) = launchers
                    .find(|launcher| self.aggregates.matrix_positions[launcher] == placement)
                {
                    changes += self.launcher_shift_sequence(
                        project,
                        launcher,
                        massive_applications::MoveDirection::Right,
                    )?;
                }
                changes <<= ProjectChange::AddLauncher {
                    project,
                    id: launch_profile_id,
                    profile,
                    placement,
                };
                changes <<= TopologyChange::Add {
                    what: launch_profile_id.into(),
                    under: DesktopTarget::ProjectMatrix(project),
                    after: None,
                };
            }
            ProjectCommand::RemoveLauncher(launch_profile_id) => {
                // If this is the last launcher of a project, remove the whole project.
                let project = self
                    .aggregates
                    .hierarchy
                    .project_of_launcher(launch_profile_id);
                if self.aggregates.hierarchy.matrix_launchers(project).count() == 1 {
                    changes += self.plan_project_removal_focus(project);
                    changes += self.plan_remove_project(project);
                    return Ok(changes);
                }

                let launcher_target = DesktopTarget::Launcher(launch_profile_id);
                if let Some(focused) = self.event_router.keyboard_focus()
                    && self
                        .aggregates
                        .hierarchy
                        .path_contains_target(Some(focused), &launcher_target)
                {
                    changes += set_focus(
                        Some(self.launcher_removal_focus(launch_profile_id, focused)),
                        KeyboardFocusReason::InputTransition,
                    );
                }

                changes += self.plan_remove_launcher(
                    project,
                    launch_profile_id,
                    Some(RemoveSlotShiftingPolicy::ShiftLeft),
                );
            }
            ProjectCommand::SetStartupProfile(launch_profile_id) => {
                changes <<= ProjectChange::SetStartupProfile(launch_profile_id)
            }
        }

        Ok(changes)
    }

    fn plan_project_removal_focus(&self, project: ProjectId) -> Changes {
        let project_target = DesktopTarget::Project(project);
        if self
            .aggregates
            .hierarchy
            .path_contains_target(self.event_router.keyboard_focus(), &project_target)
        {
            return set_focus(
                Some(self.project_removal_focus(project)),
                KeyboardFocusReason::InputTransition,
            );
        }

        Changes::Empty
    }

    fn plan_remove_project(&self, project: ProjectId) -> Changes {
        let mut changes = Changes::Empty;
        for launcher in self.aggregates.hierarchy.matrix_launchers(project) {
            changes += self.plan_remove_launcher(project, launcher, None);
        }

        changes <<= ProjectChange::RemoveProject(project);
        changes <<= TopologyChange::Remove(DesktopTarget::Project(project));
        changes
    }

    fn plan_remove_launcher(
        &self,
        project: ProjectId,
        launcher: LaunchProfileId,
        shifting_policy: Option<RemoveSlotShiftingPolicy>,
    ) -> Changes {
        let mut changes = Changes::Empty;
        for instance in self.aggregates.hierarchy.launcher_instances(launcher) {
            changes += [
                DesktopChange::Topology(TopologyChange::Remove(instance.into())),
                DesktopChange::HideInstance { launcher, instance },
                DesktopChange::ShutdownInstance(instance),
            ];
        }
        let placement = self.aggregates.matrix_positions[&launcher];
        changes <<= TopologyChange::Remove(launcher.into());
        changes <<= ProjectChange::RemoveLauncher(launcher);
        if let Some(shifting_policy) = shifting_policy {
            changes <<= ProjectChange::RemoveSlot {
                project,
                placement,
                shifting_policy,
            };
        }
        changes
    }

    pub fn apply_change(
        &mut self,
        change: DesktopChange,
        frame: &mut Frame,
        instance_manager: &mut InstanceManager,
    ) -> Result<ChangeOutput> {
        match change {
            DesktopChange::SpawnInstance {
                instance,
                root,
                mut parameters,
            } => {
                // Probably pull the name of the application into SpawnInstance?
                let application = self
                    .env
                    .applications
                    .get_named(&self.env.primary_application)
                    .context("Internal error, application not registered")?;

                parameters.insert(
                    "size_px".to_string(),
                    json!([
                        self.default_panel_size.width,
                        self.default_panel_size.height
                    ]),
                );
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
            DesktopChange::PresentInstance {
                launcher,
                initial_center_translation,
                instance,
                root,
                parameters,
            } => {
                self.present_instance(
                    launcher,
                    initial_center_translation,
                    instance,
                    root,
                    parameters,
                    frame,
                )?;
            }
            DesktopChange::HideInstance { launcher, instance } => {
                self.hide_instance(launcher, instance)?;
            }
            DesktopChange::SetFocus { target, reason } => {
                let previous_focus = self.event_router.keyboard_focus().cloned();
                self.focus(target.as_ref(), instance_manager, reason)?;
                let current_focus = self.event_router.keyboard_focus().cloned();

                let mut output = ChangeOutput::default();
                if let Some(previous_focus) = &previous_focus {
                    output.focus_changed(previous_focus.clone());
                }
                if let Some(current_focus) = &current_focus {
                    output.focus_changed(current_focus.clone());
                }

                return Ok(output);
            }
            DesktopChange::CommitNavigationAffinity(column_affinity) => {
                self.navigation_control
                    .commit_column_affinity(column_affinity);
            }
            DesktopChange::CommitFocusDepth(focus_depth) => {
                if self.focus_depth != focus_depth {
                    self.focus_depth = focus_depth;

                    let mut output = ChangeOutput::default();
                    if let Some(focused) = self.event_router.keyboard_focus() {
                        output.focus_changed(focused.clone());
                    }
                    return Ok(output);
                }
            }
            DesktopChange::WindowResized => {
                let mut output = ChangeOutput::default();
                output.surface.window_size_changed = true;
                // A window resize only affects the presentation of instances if we are in
                // [`FocusDepth::InstanceFullScreen`] and an instance is focused.
                if self.focus_depth == FocusDepth::InstanceFullScreen
                    && let Some(instance) = self.focused_path().instance()
                {
                    // Design: Somehow this is not a directly affected by a focus change. So there
                    // is a discrepancy between "updating the presentation" and a target affected by
                    // a focus change (somehow the target should probably decide about this if it's
                    // "presentation" is affected?).
                    output.measure(DesktopTarget::Instance(instance));
                }
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
                let measure_set = self.apply_topology_change(change, instance_manager)?;
                let current_focus = self.event_router.keyboard_focus().cloned();

                // DRY: This looks similar to SetFocus
                let mut output = ChangeOutput::measures(measure_set);
                if let Some(ref previous_focus) = previous_focus {
                    output.focus_changed(previous_focus.clone());
                }
                if let Some(ref current_focus) = current_focus {
                    output.focus_changed(current_focus.clone());
                }

                return Ok(output);
            }
            DesktopChange::ForwardEvents(transitions) => {
                let commands = self.forward_event_transitions(transitions, instance_manager)?;
                let mut changes = Changes::default();
                for command in commands {
                    changes += self.plan(command, frame.scene())?;
                }
                return Ok(ChangeOutput::changes(changes));
            }
            DesktopChange::IntegrateInstanceSubmission(instance_id, instance_submission) => {
                return self.apply_instance_submission(instance_id, instance_submission, frame);
            }
            DesktopChange::Project(project_change) => {
                return self.apply_project_change(project_change, frame);
            }
        }

        Ok(ChangeOutput::default())
    }

    pub fn apply_topology_change(
        &mut self,
        change: TopologyChange,
        instance_manager: &InstanceManager,
    ) -> Result<TargetSet> {
        match change {
            TopologyChange::Add { what, under, after } => {
                if let Some(after) = after {
                    // Design: `under` can be resolved via `after`!
                    self.aggregates.hierarchy.add_after(after, what)?;
                } else {
                    self.aggregates.hierarchy.add(under.clone(), what)?;
                }
                Ok(under.into())
            }
            TopologyChange::AddNested { what, under } => {
                self.aggregates.hierarchy.add_nested(under.clone(), what)?;
                Ok(under.into())
            }
            TopologyChange::Insert {
                what,
                at_index,
                under,
            } => {
                self.aggregates
                    .hierarchy
                    .insert_at(under.clone(), at_index, what)?;
                Ok(under.into())
            }
            TopologyChange::Remove(target) => {
                // A removed subtree may still hold pointer and/or keyboard focus. Clear pointer
                // focus and retarget keyboard focus to the parent before removal so the event
                // router is not left pointing at a removed node.
                self.unfocus_pointer_if_path_contains(&target, instance_manager)?;
                self.refocus_to_parent_if_path_contains(&target, instance_manager)?;
                Ok(self.remove_target(&target)?)
            }
        }
    }

    fn apply_project_change(
        &mut self,
        change: ProjectChange,
        frame: &mut Frame,
    ) -> Result<ChangeOutput> {
        match change {
            ProjectChange::AddProject { id, properties } => {
                let parent_location = self.desktop_presenter.location.clone();
                self.aggregates.projects.insert(
                    id,
                    ProjectPresenter::new(
                        properties,
                        parent_location,
                        frame.scene(),
                        &mut self.fonts.lock(),
                        frame.movement_runtime(),
                    ),
                )?;
            }
            ProjectChange::RemoveProject(project) => {
                self.aggregates.projects.remove(&project)?;
            }
            ProjectChange::AddLauncher {
                project,
                id,
                profile,
                placement,
            } => {
                let launchers = self.aggregates.hierarchy.matrix_launchers(project);
                self.aggregates
                    .matrix_positions
                    .place(launchers, id, placement)?;

                let matrix_location = self
                    .aggregates
                    .projects
                    .get(&project)
                    .expect("Project missing")
                    .matrix
                    .location();

                let presenter = LauncherPresenter::new(
                    matrix_location,
                    id,
                    profile,
                    massive_geometry::Size::default(),
                    frame.scene(),
                    &mut self.fonts.lock(),
                    frame.movement_runtime(),
                );
                self.aggregates.launchers.insert(id, presenter)?;
            }
            ProjectChange::MoveLauncher {
                launcher,
                placement,
            } => {
                let project = self.aggregates.hierarchy.project_of_launcher(launcher);
                *self
                    .aggregates
                    .matrix_positions
                    .get_mut(&launcher)
                    .expect("Matrix position missing for launcher") = placement;
                return Ok(ChangeOutput::measures(DesktopTarget::ProjectMatrix(
                    project,
                )));
            }
            ProjectChange::RemoveLauncher(launch_profile_id) => {
                self.aggregates.launchers.remove(&launch_profile_id)?;
                self.aggregates
                    .matrix_positions
                    .remove(&launch_profile_id)?;
            }
            ProjectChange::RemoveSlot {
                project,
                placement,
                shifting_policy,
            } => {
                let launchers = self.aggregates.hierarchy.matrix_launchers(project);
                self.aggregates
                    .matrix_positions
                    .remove_slot(launchers, placement, shifting_policy);
                return Ok(ChangeOutput::measures(DesktopTarget::ProjectMatrix(
                    project,
                )));
            }
            ProjectChange::SetStartupProfile(launch_profile_id) => {
                self.aggregates.startup_profile = launch_profile_id;
            }
        }

        Ok(ChangeOutput::default())
    }

    fn apply_instance_submission(
        &mut self,
        instance: InstanceId,
        submission: InstanceSubmission,
        frame: &mut Frame,
    ) -> Result<ChangeOutput> {
        let (changes, pacing) = submission.into_parts();
        let mut output = ChangeOutput::default();

        for change in changes.release() {
            output.combine(self.apply_instance_change(instance, change, frame)?);
        }

        self.set_instance_pacing(instance, pacing);
        Ok(output)
    }

    fn apply_instance_change(
        &mut self,
        instance: InstanceId,
        change: InstanceChange,
        frame: &mut Frame,
    ) -> Result<ChangeOutput> {
        match change {
            InstanceChange::Scene(change) => {
                frame.push_change(change);
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
            .project_of_target(&instance.into())
            .expect("Instance has no project");
        match &request {
            ConfigurationRequest::AddProject => {
                let project = ProjectId::new();
                let launcher = LaunchProfileId::new();

                // ADR: Decided to add a bare launcher if a new project is added, so that we can
                // enter it and add further launchers from there.

                let commands = [
                    ProjectCommand::AddProject {
                        id: project,
                        properties: ProjectProperties {
                            name: DEFAULT_NEW_PROJECT_NAME.to_string(),
                        },
                        after: Some(current_project),
                    },
                    ProjectCommand::AddLauncher {
                        project,
                        id: launcher,
                        profile: LaunchProfile {
                            name: DEFAULT_NEW_LAUNCHER_NAME.to_string(),
                            mode: LauncherMode::Visor,
                            tags: Vec::new(),
                            params: Default::default(),
                        },
                        placement: MatrixPlacement { column: 0, row: 0 },
                    },
                ];

                let mut changes = Changes::Empty;
                for command in commands {
                    changes += self.plan_project(command)?;
                }

                Ok(ChangeOutput::changes(changes))
            }
            ConfigurationRequest::RemoveProject { name } => {
                let project = match name {
                    Some(name) => {
                        let Some(project) = self
                            .aggregates
                            .hierarchy
                            .get_nested(&DesktopTarget::Desktop)
                            .iter()
                            .find_map(|target| match target {
                                DesktopTarget::Project(project)
                                    if self.aggregates.projects[project].name() == name =>
                                {
                                    Some(*project)
                                }
                                _ => None,
                            })
                        else {
                            warn!("Project '{name}' not found");
                            return Ok(ChangeOutput::default());
                        };
                        project
                    }
                    None => current_project,
                };

                Ok(ChangeOutput::changes(
                    self.plan_project(ProjectCommand::RemoveProject(project))?,
                ))
            }
            ConfigurationRequest::AddLauncher => {
                let current_launcher = self.aggregates.hierarchy.launcher_of_instance(instance);
                let current_placement = self.aggregates.matrix_positions[&current_launcher];

                let changes = self.plan_project(ProjectCommand::AddLauncher {
                    project: current_project,
                    id: LaunchProfileId::new(),
                    profile: LaunchProfile {
                        name: DEFAULT_NEW_LAUNCHER_NAME.to_string(),
                        mode: LauncherMode::Visor,
                        tags: Vec::new(),
                        params: Default::default(),
                    },
                    placement: MatrixPlacement {
                        column: current_placement.column + 1,
                        row: current_placement.row,
                    },
                })?;

                Ok(ChangeOutput::changes(changes))
            }
            ConfigurationRequest::RemoveLauncher { name } => {
                let launcher = match name {
                    Some(name) => {
                        // ADR, stay on the project for now.
                        let Some(launcher) = self
                            .aggregates
                            .hierarchy
                            .matrix_launchers(current_project)
                            .find(|launcher| self.aggregates.launchers[launcher].name() == name)
                        else {
                            warn!("Launcher '{name}' not found in the current project");
                            return Ok(ChangeOutput::default());
                        };
                        launcher
                    }
                    None => self.aggregates.hierarchy.launcher_of_instance(instance),
                };

                Ok(ChangeOutput::changes(
                    self.plan_project(ProjectCommand::RemoveLauncher(launcher))?,
                ))
            }
            ConfigurationRequest::MoveLauncher { direction } => {
                let launcher = self.aggregates.hierarchy.launcher_of_instance(instance);
                let current_placement = self.aggregates.matrix_positions[&launcher];
                let placement = MatrixPositions::moved_placement(current_placement, *direction);
                let Some(placement) = placement else {
                    warn!(
                        "Ignoring {direction:?} launcher move from matrix position ({}, {})",
                        current_placement.column, current_placement.row,
                    );
                    return Ok(ChangeOutput::default());
                };
                let swapped_launcher = self
                    .aggregates
                    .hierarchy
                    .matrix_launchers(current_project)
                    .find(|candidate| self.aggregates.matrix_positions[candidate] == placement);
                let mut changes = Changes::Empty;
                if let Some(swapped_launcher) = swapped_launcher {
                    changes <<= ProjectChange::MoveLauncher {
                        launcher: swapped_launcher,
                        placement: current_placement,
                    };
                }
                changes <<= ProjectChange::MoveLauncher {
                    launcher,
                    placement,
                };
                Ok(ChangeOutput::changes(changes))
            }
            ConfigurationRequest::PushLauncher { direction } => {
                let launcher = self.aggregates.hierarchy.launcher_of_instance(instance);
                let current_placement = self.aggregates.matrix_positions[&launcher];
                match self.launcher_shift_sequence(current_project, launcher, *direction) {
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
            ConfigurationRequest::Resize { size_px } => {
                let mut changes = Changes::Empty;

                // If we are in fullscreen, show the changes by resetting the zoom level, otherwise
                // the user would see nothing.
                if self.focus_depth == FocusDepth::InstanceFullScreen {
                    changes <<= DesktopChange::CommitFocusDepth(FocusDepth::Instance);
                }

                changes <<= DesktopChange::ResizeAll((*size_px).into());

                Ok(ChangeOutput::changes(changes))
            }
            ConfigurationRequest::Undo => todo!(),
            ConfigurationRequest::Redo => todo!(),
        }
    }

    fn launcher_shift_sequence(
        &self,
        project: ProjectId,
        launcher: LaunchProfileId,
        direction: massive_applications::MoveDirection,
    ) -> Result<Changes> {
        let launchers = self.aggregates.hierarchy.matrix_launchers(project);
        let shifted_launchers = self
            .aggregates
            .matrix_positions
            .shifted_launchers(launchers, launcher, direction)?;

        let mut changes = Changes::Empty;
        for (launcher, placement) in shifted_launchers {
            changes <<= ProjectChange::MoveLauncher {
                launcher,
                placement,
            };
        }
        Ok(changes)
    }
}

const DEFAULT_NEW_PROJECT_NAME: &str = "New Project";
const DEFAULT_NEW_LAUNCHER_NAME: &str = "New Launcher";
