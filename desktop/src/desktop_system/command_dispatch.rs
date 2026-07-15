use anyhow::{Context, Result};
use log::{debug, warn};

use massive_applications::{
    CreationMode, DesktopRequest, InstanceChange, InstanceId, InstanceSubmission, ViewChange,
    ViewRole,
};
use massive_shell::Scene;

use super::{DesktopCommand, DesktopSystem, DesktopTarget, KeyboardFocusReason};
use crate::ShiftingPolicy;
use crate::desktop_system::change::{
    Changes, DesktopChange, ProjectChange, TopologyChange, set_focus,
};
use crate::desktop_system::effects::MeasureSet;
use crate::desktop_system::navigation::focus_depth_from_target;
use crate::desktop_system::{ProjectCommand, UserState};
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::instance_presenter::InstanceRoot;
use crate::projects::{
    LaunchProfile, LaunchProfileId, LauncherMode, LauncherPresenter, MatrixPlacement, ProjectId,
    ProjectPresenter, ProjectProperties,
};

/// The outcome of applying a change: the measures it produced and any follow-up changes.
#[derive(Debug, Default)]
pub struct ChangeOutput {
    /// Additional changes to schedule.
    pub changes: Changes,
    pub measures: MeasureSet,
}
impl ChangeOutput {
    /// An outcome that produced the given measures.
    fn measures(measures: MeasureSet) -> Self {
        Self {
            measures,
            ..Self::default()
        }
    }

    pub fn changes(changes: Changes) -> Self {
        Self {
            changes,
            ..Self::default()
        }
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
                changes += DesktopChange::SetUserState(UserState::default());

                return Ok(changes);
            }
            DesktopCommand::StopInstance(instance) => {
                let launcher = self
                    .aggregates
                    .hierarchy
                    .launcher_of_instance(instance)
                    .expect("Launcher not found");

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
                changes += DesktopChange::SetUserState(UserState::default());

                return Ok(changes);
            }
            DesktopCommand::Navigate(direction) => return self.plan_navigate(direction),
            DesktopCommand::ZoomIn => {
                if let Some(focus_depth) = self.user_state.focus_depth.zoom_in() {
                    let user_state = UserState { focus_depth };
                    return Ok(DesktopChange::SetUserState(user_state).into());
                }
            }
            DesktopCommand::ZoomOut => {
                if let Some(focus_depth) = self.user_state.focus_depth.zoom_out() {
                    let user_state = UserState { focus_depth };
                    return Ok(DesktopChange::SetUserState(user_state).into());
                }
            }
            DesktopCommand::ResetZoom => {
                if let Some(keyboard_focus) = self.event_router.keyboard_focus() {
                    let current_level = self.user_state.focus_depth;
                    let keyboard_focus_level = focus_depth_from_target(keyboard_focus);

                    if current_level != keyboard_focus_level {
                        let mut new_user_state = self.user_state.clone();
                        new_user_state.focus_depth = keyboard_focus_level;
                        return Ok(DesktopChange::SetUserState(new_user_state).into());
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
                changes <<= TopologyChange::Remove(DesktopTarget::Project(project_id));
                changes <<= ProjectChange::RemoveProject(project_id);
            }
            ProjectCommand::AddLauncher {
                project,
                id: launch_profile_id,
                profile,
                placement,
            } => {
                let launchers = self.aggregates.hierarchy.matrix_launchers(project);
                if !self
                    .aggregates
                    .matrix_positions
                    .is_available(launchers, placement)
                {
                    changes <<= ProjectChange::MakeSlotAvailable { project, placement };
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
                changes <<= TopologyChange::Remove(launch_profile_id.into());
                changes <<= ProjectChange::RemoveLauncher(launch_profile_id);
            }
            ProjectCommand::SetStartupProfile(launch_profile_id) => {
                changes <<= ProjectChange::SetStartupProfile(launch_profile_id)
            }
        }

        Ok(changes)
    }

    pub fn apply_change(
        &mut self,
        change: DesktopChange,
        scene: &Scene,
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
                    root.location(),
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
                    scene,
                )?;
            }
            DesktopChange::HideInstance { launcher, instance } => {
                self.hide_instance(launcher, instance)?;
            }
            DesktopChange::SetFocus { target, reason } => {
                self.focus(target.as_ref(), instance_manager, reason)?;
            }
            DesktopChange::SetNavigationAffinity(column_affinity) => {
                self.navigation_control
                    .commit_column_affinity(column_affinity);
            }
            DesktopChange::SetUserState(user_state) => {
                self.user_state = user_state;
            }
            DesktopChange::Topology(change) => {
                let measure_set = self.apply_topology_change(change, instance_manager)?;
                return Ok(ChangeOutput::measures(measure_set));
            }
            DesktopChange::ForwardEvents(transitions) => {
                let commands = self.forward_event_transitions(transitions, instance_manager)?;
                let mut changes = Changes::default();
                for command in commands {
                    changes += self.plan(command, scene)?;
                }
                return Ok(ChangeOutput::changes(changes));
            }
            DesktopChange::IntegrateInstanceSubmission(instance_id, instance_submission) => {
                return self.apply_instance_submission(instance_id, instance_submission, scene);
            }
            DesktopChange::Project(project_change) => {
                return self.apply_project_change(project_change, scene);
            }
        }

        Ok(ChangeOutput::default())
    }

    pub fn apply_topology_change(
        &mut self,
        change: TopologyChange,
        instance_manager: &InstanceManager,
    ) -> Result<MeasureSet> {
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
        scene: &Scene,
    ) -> Result<ChangeOutput> {
        match change {
            ProjectChange::AddProject { id, properties } => {
                let parent_location = self.desktop_presenter.location.clone();
                self.aggregates.projects.insert(
                    id,
                    ProjectPresenter::new(
                        properties,
                        parent_location,
                        scene,
                        &mut self.fonts.lock(),
                    ),
                )?;
            }
            ProjectChange::RemoveProject(project) => {
                self.aggregates.projects.remove(&project)?;
            }
            // Keep matrix placement and removal in launcher lifecycle changes so they carry
            // complete state instead of requiring an ordered sequence of partial changes.
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
                    scene,
                    &mut self.fonts.lock(),
                );
                self.aggregates.launchers.insert(id, presenter)?;
            }
            ProjectChange::RemoveLauncher(launch_profile_id) => {
                self.aggregates.launchers.remove(&launch_profile_id)?;
                self.aggregates
                    .matrix_positions
                    .remove(&launch_profile_id)?;
            }
            ProjectChange::MakeSlotAvailable { project, placement } => {
                let launchers = self.aggregates.hierarchy.matrix_launchers(project);
                self.aggregates.matrix_positions.make_slot_available(
                    launchers,
                    placement,
                    ShiftingPolicy::ShiftRight,
                )?;
                return Ok(ChangeOutput::measures(
                    DesktopTarget::ProjectMatrix(project).into(),
                ));
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
        scene: &Scene,
    ) -> Result<ChangeOutput> {
        let (changes, pacing) = submission.into_parts();
        let mut measures = MeasureSet::Empty;
        let mut follow_ups = Changes::Empty;

        for change in changes.release() {
            let outcome = self.apply_instance_change(instance, change, scene)?;
            measures += outcome.measures;
            follow_ups += outcome.changes;
        }

        self.set_instance_pacing(instance, pacing);
        Ok(ChangeOutput {
            changes: follow_ups,
            measures,
        })
    }

    fn apply_instance_change(
        &mut self,
        instance: InstanceId,
        change: InstanceChange,
        scene: &Scene,
    ) -> Result<ChangeOutput> {
        match change {
            InstanceChange::Scene(change) => {
                scene.push_change(change);
                Ok(ChangeOutput::default())
            }
            InstanceChange::CreateView(creation_info) => {
                let mut output = self.present_view(instance, &creation_info, scene)?;

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
            InstanceChange::Desktop(request) => self.handle_desktop_request(instance, request),
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

    fn handle_desktop_request(
        &self,
        instance: InstanceId,
        request: DesktopRequest,
    ) -> Result<ChangeOutput> {
        let current_project = self
            .aggregates
            .hierarchy
            .project_of_target(&instance.into())
            .expect("Instance has not project?");
        match &request {
            DesktopRequest::AddProject => {
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
            DesktopRequest::RemoveProject { name: _ } => todo!(),
            DesktopRequest::AddLauncher => {
                let current_launcher = self
                    .aggregates
                    .hierarchy
                    .launcher_of_instance(instance)
                    .expect("Instance has no launcher");
                let current_placement = self.aggregates.matrix_positions[&current_launcher];

                Ok(ChangeOutput::changes(self.plan_project(
                    ProjectCommand::AddLauncher {
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
                    },
                )?))
            }
            DesktopRequest::RemoveLauncher { name: _ } => todo!(),
            DesktopRequest::Undo => todo!(),
            DesktopRequest::Redo => todo!(),
        }
    }
}

const DEFAULT_NEW_PROJECT_NAME: &str = "New Project";
const DEFAULT_NEW_LAUNCHER_NAME: &str = "New Launcher";
