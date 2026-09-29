use anyhow::{Context, Result, ensure};
use log::{debug, warn};
use serde_json::json;

use super::change::Zoom;
use super::change::set_focus;
use super::change::{Changes, ConfigurationChange, DesktopChange, TopologyChange};
use super::navigation::focus_depth_from_target;
use super::{
    ChangeSurface, DesktopCommand, DesktopSystem, DesktopTarget, FocusDepth, KeyboardFocusReason,
    ProjectCommand, TransactionEffectsMode,
};
use crate::desktop_system::change_surface::TargetSet;
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::instance_presenter::InstanceRoot;
use crate::projects::{
    LaunchProfile, LaunchProfileId, LauncherMode, LauncherPresenter, MatrixPlacement, ProjectId,
    ProjectPresenter,
};

use massive_applications::prelude::*;
use massive_applications::{
    ConfigurationRequest, CreationMode, InstanceChange, InstanceId, InstanceSubmission, ViewChange,
    ViewEvent, ViewRole,
};

/// Which slot-removal shifting `plan_remove_launcher` emits: the launchers right
/// of the freed slot move one column left, as explicit `MoveLauncher` changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoveSlotShiftingPolicy {
    ShiftLeft,
}

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
                    // Rare case: only a spawned instance needs new scene objects, so the ambient
                    // change queue is used here instead of being threaded through `plan`.
                    None => (InstanceRoot::new(), true),
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
            ProjectCommand::AddProject { id, name, after } => {
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
                changes <<= ConfigurationChange::AddProject { id, name };
            }
            ProjectCommand::RemoveProject(project_id) => {
                // The session boots into a configuration-defined launcher, so the
                // last one cannot be removed. Planned here, where the whole command
                // is still visible, so a rejection emits no change at all.
                let launchers = self.aggregates.configuration.launcher_count();
                let removed = self
                    .aggregates
                    .configuration
                    .project(project_id)
                    .map(|project| project.launchers().len())
                    .unwrap_or(0);
                ensure!(
                    launchers > removed,
                    "Configuration must define at least one launcher"
                );

                changes += self.plan_project_removal_focus(project_id);
                changes += self.plan_remove_project(project_id);
            }
            ProjectCommand::AddLauncher {
                project,
                id: launch_profile_id,
                profile,
                placement,
            } => {
                let profile = LaunchProfile {
                    name: profile.name,
                    ..profile
                };
                let mut launchers = self.aggregates.hierarchy.matrix_launchers(project);
                if let Some(launcher) = launchers.find(|launcher| {
                    self.aggregates.configuration.placement_of(*launcher) == Some(placement)
                }) {
                    changes += self.launcher_shift_sequence(
                        project,
                        launcher,
                        massive_applications::MoveDirection::Right,
                    )?;
                }
                changes <<= ConfigurationChange::AddLauncher {
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
                    // Removing the last launcher removes its project, so the
                    // configuration must hold a launcher beyond this project's.
                    ensure!(
                        self.aggregates.configuration.launcher_count() > 1,
                        "Configuration must define at least one launcher"
                    );
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
            ProjectCommand::SetStartupLauncher(launch_profile_id) => {
                changes <<= ConfigurationChange::SetStartupLauncher(launch_profile_id)
            }
        }

        Ok(changes)
    }

    /// Names a new project: a default name gets the lowest index not already in
    /// use, while a user-chosen name is taken as it is — duplicate names are
    /// allowed. An id the configuration already holds is the boot flow re-applying
    /// the names it parsed, which must pass through unchanged.
    #[allow(dead_code)]
    fn default_project_name(&self, id: ProjectId, name: &str) -> String {
        if name != DEFAULT_NEW_PROJECT_NAME || self.aggregates.configuration.project(id).is_some() {
            return name.to_string();
        }
        let existing: Vec<&str> = self
            .aggregates
            .configuration
            .projects()
            .iter()
            .map(|project| project.name())
            .collect();
        indexed_default_name(name, &existing)
    }

    /// The launcher counterpart of [`Self::default_project_name`], indexed among the
    /// siblings of the launcher's project.
    #[allow(dead_code)]
    fn default_launcher_name(&self, id: LaunchProfileId, name: &str) -> String {
        if name != DEFAULT_NEW_LAUNCHER_NAME || self.aggregates.configuration.launcher(id).is_some()
        {
            return name.to_string();
        }
        let existing: Vec<&str> = self
            .aggregates
            .configuration
            .project_of_launcher(id)
            .map(|project| {
                project
                    .launchers()
                    .iter()
                    .map(|launcher| launcher.name())
                    .collect()
            })
            .unwrap_or_default();
        indexed_default_name(name, &existing)
    }

    /// The project owning the keyboard-focused launcher, when it is still in the
    /// configuration.
    fn focused_project(&self) -> Option<ProjectId> {
        let focused = self.event_router.keyboard_focus()?;
        self.aggregates
            .hierarchy
            .project_of_target(focused)
            .filter(|project| self.aggregates.configuration.project(*project).is_some())
    }

    /// The project called `name` that sits nearest — in document order — to the
    /// focused launcher's project. Duplicate names address the nearest, and with no
    /// focus or name in it the first match answers; `None` when no project is so
    /// named.
    fn nearest_project(&self, name: &str) -> Option<ProjectId> {
        let projects = self.aggregates.configuration.projects();
        let focused = self
            .focused_project()
            .and_then(|project| self.aggregates.configuration.project_index(project));
        projects
            .iter()
            .enumerate()
            .filter(|(_, project)| project.name() == name)
            .map(|(index, project)| (index.abs_diff(focused.unwrap_or(index)), project.id()))
            .min_by_key(|(distance, _)| *distance)
            .map(|(_, id)| id)
    }

    /// The launcher called `name` in `project` that sits nearest — in matrix
    /// distance — to the keyboard-focused launcher. Duplicate names address the
    /// nearest, and with no focused launcher in `project` the first match answers;
    /// `None` when no launcher of `project` is so named.
    fn nearest_launcher(&self, project: ProjectId, name: &str) -> Option<LaunchProfileId> {
        let focused = self
            .event_router
            .keyboard_focus()
            .and_then(|focused| self.aggregates.hierarchy.launcher_of_target(focused))
            .filter(|launcher| self.aggregates.hierarchy.project_of_launcher(*launcher) == project)
            .and_then(|launcher| self.aggregates.configuration.launcher_index(launcher));
        let focused = focused.unwrap_or(0);
        self.aggregates
            .configuration
            .launchers_sorted(project)
            .iter()
            .enumerate()
            .filter(|(_, launcher)| launcher.name() == name)
            .map(|(index, launcher)| (index.abs_diff(focused), launcher.id()))
            .min_by_key(|(distance, _)| *distance)
            .map(|(_, id)| id)
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

        changes <<= ConfigurationChange::RemoveProject(project);
        changes <<= TopologyChange::Remove(DesktopTarget::Project(project));
        changes
    }

    /// Removes a launcher from the matrix, shifting the launchers right of the freed
    /// slot one column left (the `ShiftLeft` slot-removal policy) as explicit move
    /// changes, so the file mirrors each launcher's resulting slot.
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
        let placement = self
            .aggregates
            .configuration
            .placement_of(launcher)
            .expect("Matrix position missing for launcher");
        changes <<= TopologyChange::Remove(launcher.into());
        changes <<= ConfigurationChange::RemoveLauncher(launcher);
        if shifting_policy == Some(RemoveSlotShiftingPolicy::ShiftLeft) {
            for (launcher, placement) in self
                .aggregates
                .configuration
                .shifted_left_launchers(project, placement)
            {
                changes <<= ConfigurationChange::MoveLauncher {
                    launcher,
                    placement,
                };
            }
        }
        changes
    }

    pub fn apply_change(
        &mut self,
        change: DesktopChange,
        instance_manager: &mut InstanceManager,
        effects_mode: TransactionEffectsMode,
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
                output.focus_changed(previous_focus, current_focus);

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
                        output.measure(focused.clone());
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
                // A setup change only updates the live model; it must not mirror into
                // the persisted document.
                if effects_mode != TransactionEffectsMode::Setup {
                    self.configuration.apply(project_change.clone())?;
                }
                return self.apply_project_change(project_change);
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
            ConfigurationChange::AddProject { id, name } => {
                let parent_location = self.desktop_presenter.location.clone();
                let presenter = ProjectPresenter::new(name.clone(), parent_location);
                self.aggregates.projects.insert(id, presenter)?;
                self.aggregates.configuration.add_project(id, name);
            }
            ConfigurationChange::RemoveProject(project) => {
                self.aggregates.projects.remove(&project)?;
                self.aggregates.configuration.remove_project(project);
            }
            ConfigurationChange::AddLauncher {
                project,
                id,
                profile,
                placement,
            } => {
                // Aggregate first, then views: the presenter's construction-time name
                // glyph reads back from the aggregate the change just landed in.
                self.aggregates
                    .configuration
                    .add_launcher(project, id, profile.clone(), placement);

                let name = self.aggregates.configuration[id].name().to_string();
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
                    name,
                    massive_geometry::Size::default(),
                );
                self.aggregates.launchers.insert(id, presenter)?;
            }
            ConfigurationChange::MoveLauncher {
                launcher,
                placement,
            } => {
                let project = self.aggregates.hierarchy.project_of_launcher(launcher);
                self.aggregates
                    .configuration
                    .move_launcher(launcher, placement);
                return Ok(ChangeOutput::measures(DesktopTarget::ProjectMatrix(
                    project,
                )));
            }
            ConfigurationChange::RemoveLauncher(launch_profile_id) => {
                self.aggregates.launchers.remove(&launch_profile_id)?;
                self.aggregates
                    .configuration
                    .remove_launcher(launch_profile_id);
            }
            // The startup launcher is consumed at boot (`Setup`); the runtime model
            // does not retain it. Only this dispatch must handle it.
            ConfigurationChange::SetStartupLauncher(_) => {}
        }

        Ok(ChangeOutput::default())
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
                        name: DEFAULT_NEW_PROJECT_NAME.to_string(),
                        after: Some(current_project),
                    },
                    ProjectCommand::AddLauncher {
                        project,
                        id: launcher,
                        profile: LaunchProfile {
                            name: DEFAULT_NEW_LAUNCHER_NAME.to_string(),
                            mode: LauncherMode::Visor,
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
                    Some(name) => match self.nearest_project(name) {
                        Some(project) => project,
                        None => {
                            warn!("Project '{name}' not found");
                            return Ok(ChangeOutput::default());
                        }
                    },
                    None => current_project,
                };

                Ok(ChangeOutput::changes(
                    self.plan_project(ProjectCommand::RemoveProject(project))?,
                ))
            }
            ConfigurationRequest::AddLauncher => {
                let current_launcher = self.aggregates.hierarchy.launcher_of_instance(instance);
                let current_placement = self
                    .aggregates
                    .configuration
                    .placement_of(current_launcher)
                    .expect("Focused launcher has no matrix placement");

                let changes = self.plan_project(ProjectCommand::AddLauncher {
                    project: current_project,
                    id: LaunchProfileId::new(),
                    profile: LaunchProfile {
                        name: DEFAULT_NEW_LAUNCHER_NAME.to_string(),
                        mode: LauncherMode::Visor,
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
                        match self.nearest_launcher(current_project, name) {
                            Some(launcher) => launcher,
                            None => {
                                warn!("Launcher '{name}' not found in the current project");
                                return Ok(ChangeOutput::default());
                            }
                        }
                    }
                    None => self.aggregates.hierarchy.launcher_of_instance(instance),
                };

                Ok(ChangeOutput::changes(
                    self.plan_project(ProjectCommand::RemoveLauncher(launcher))?,
                ))
            }
            ConfigurationRequest::MoveLauncher { direction } => {
                let launcher = self.aggregates.hierarchy.launcher_of_instance(instance);
                let current_placement = self
                    .aggregates
                    .configuration
                    .placement_of(launcher)
                    .expect("Focused launcher has no matrix placement");
                let placement = current_placement.moved_placement(*direction);
                let Some(placement) = placement else {
                    warn!(
                        "Ignoring {direction:?} launcher move from matrix position ({}, {})",
                        current_placement.column, current_placement.row,
                    );
                    return Ok(ChangeOutput::default());
                };
                let swapped_launcher = self
                    .aggregates
                    .configuration
                    .launcher_at(current_project, placement);
                let mut changes = Changes::Empty;
                if let Some(swapped_launcher) = swapped_launcher {
                    changes <<= ConfigurationChange::MoveLauncher {
                        launcher: swapped_launcher,
                        placement: current_placement,
                    };
                }
                changes <<= ConfigurationChange::MoveLauncher {
                    launcher,
                    placement,
                };
                Ok(ChangeOutput::changes(changes))
            }
            ConfigurationRequest::PushLauncher { direction } => {
                let launcher = self.aggregates.hierarchy.launcher_of_instance(instance);
                let current_placement = self
                    .aggregates
                    .configuration
                    .placement_of(launcher)
                    .expect("Focused launcher has no matrix placement");
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
        let shifted_launchers = self
            .aggregates
            .configuration
            .shifted_launchers(project, launcher, direction)?;

        let mut changes = Changes::Empty;
        for (launcher, placement) in shifted_launchers {
            changes <<= ConfigurationChange::MoveLauncher {
                launcher,
                placement,
            };
        }
        Ok(changes)
    }
}

const DEFAULT_NEW_PROJECT_NAME: &str = "New Project";
const DEFAULT_NEW_LAUNCHER_NAME: &str = "New Launcher";

/// The default name with the lowest index that is not already taken among
/// `existing`. The index only disambiguates the default name; the number is
/// reused once a previous holder is renamed or removed.
#[allow(dead_code)]
fn indexed_default_name(name: &str, existing: &[&str]) -> String {
    let mut index = 2;
    loop {
        let candidate = format!("{name} {index}");
        if !existing.contains(&candidate.as_str()) {
            return candidate;
        }
        index += 1;
    }
}
