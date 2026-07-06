use anyhow::{Context, Result};
use log::{debug, warn};

use massive_applications::{
    CreationMode, InstanceChange, InstanceId, InstanceSubmission, ViewChange, ViewRole,
};
use massive_shell::Scene;

use super::{DesktopCommand, DesktopSystem, DesktopTarget, FocusReason};
use crate::desktop_system::change::{Changes, DesktopChange, ProjectChange, TopologyChange};
use crate::desktop_system::effects::MeasureSet;
use crate::desktop_system::zoom_navigation::{zoom_in, zoom_out};
use crate::desktop_system::{ProjectCommand, UserState};
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::instance_presenter::InstanceRoot;
use crate::projects::{LauncherPresenter, ProjectPresenter};

/// The outcome of applying a change: the measures it produced and the resulting user state.
#[derive(Debug, Default)]
pub struct ChangeOutput {
    /// Additional changes to schedule.
    pub changes: Changes,
    pub measures: MeasureSet,
    pub user_state: UserState,
}

impl ChangeOutput {
    /// An outcome that produced the given measures and UserState.
    fn new(measures: MeasureSet, user_state: UserState) -> Self {
        ChangeOutput {
            changes: Changes::Empty,
            measures,
            user_state,
        }
    }

    pub fn changes(changes: Changes) -> Self {
        Self {
            changes,
            ..Self::default()
        }
    }

    /// An outcome that produced no measures.
    fn measureless(user_state: UserState) -> Self {
        ChangeOutput::new(MeasureSet::Empty, user_state)
    }

    /// Combine with a follow-up outcome: measures accumulate, user state is last-wins.
    fn combine(mut self, follow_up: Self) -> Self {
        self.changes += follow_up.changes;
        self.measures += follow_up.measures;
        self.user_state = follow_up.user_state;
        self
    }
}

impl DesktopSystem {
    /// Plan the execution of a command.
    pub fn plan(&self, command: DesktopCommand, scene: &Scene) -> Result<Changes> {
        match command {
            DesktopCommand::Project(project_command) => {
                return self.plan_project(project_command);
            }
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
                        parameters,
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
                    },
                    DesktopChange::Topology(TopologyChange::Insert {
                        what: instance.into(),
                        at_index: insertion_pos,
                        under: launcher.into(),
                    }),
                    DesktopChange::SetFocus {
                        target: Some(DesktopTarget::Instance(instance)),
                        reason: FocusReason::PresentInstance,
                    },
                ];

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
                let replacement_focus = self.event_router.focused().and_then(|focused| {
                    self.aggregates
                        .hierarchy
                        .resolve_replacement_focus_for_stopping_instance(focused, instance)
                });

                let mut changes = Changes::Empty;
                if let Some(focus) = replacement_focus {
                    changes += DesktopChange::SetFocus {
                        target: Some(focus),
                        reason: FocusReason::StopInstanceReplacement,
                    };
                }
                changes += [
                    DesktopChange::Topology(TopologyChange::Remove(instance.into())),
                    DesktopChange::HideInstance { launcher, instance },
                    DesktopChange::ShutdownInstance(instance),
                ];

                return Ok(changes);
            }
            DesktopCommand::ZoomIn => {}
            DesktopCommand::ZoomOut => {}
            // DesktopCommand::NavigateTo(_) => {}
            DesktopCommand::Navigate(_) => {}
        }

        Ok(Changes::Empty)
    }

    fn plan_project(&self, command: ProjectCommand) -> Result<Changes> {
        let mut changes = Changes::Empty;
        match command {
            ProjectCommand::AddProject { id, properties } => {
                let parent_target = DesktopTarget::Desktop;
                let project_target = DesktopTarget::Project(id);
                changes.push(TopologyChange::Add {
                    what: project_target.clone(),
                    under: parent_target,
                });
                changes.push(TopologyChange::AddNested {
                    what: [
                        DesktopTarget::ProjectHeader(id),
                        DesktopTarget::ProjectMatrix(id),
                    ]
                    .into(),
                    under: project_target,
                });
                changes.push(ProjectChange::AddProject { id, properties });
            }
            ProjectCommand::RemoveProject(project_id) => {
                changes.push(TopologyChange::Remove(DesktopTarget::Project(project_id)));
                changes.push(ProjectChange::RemoveProject(project_id));
            }
            ProjectCommand::AddLauncher {
                project,
                id: launch_profile_id,
                profile,
                placement,
            } => {
                changes.push(ProjectChange::AddLauncher {
                    project,
                    id: launch_profile_id,
                    profile,
                    placement,
                });
                changes.push(TopologyChange::Add {
                    what: launch_profile_id.into(),
                    under: DesktopTarget::ProjectMatrix(project),
                });
            }
            ProjectCommand::RemoveLauncher(launch_profile_id) => {
                changes.push(TopologyChange::Remove(launch_profile_id.into()));
                changes.push(ProjectChange::RemoveLauncher(launch_profile_id));
            }
            ProjectCommand::SetStartupProfile(launch_profile_id) => {
                changes.push(ProjectChange::SetStartupProfile(launch_profile_id))
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
            } => {
                self.present_instance(launcher, initial_center_translation, instance, root, scene)?;
            }
            DesktopChange::HideInstance { launcher, instance } => {
                self.hide_instance(launcher, instance)?;
            }
            DesktopChange::SetFocus { target, reason } => {
                self.focus(target.as_ref(), instance_manager, reason)?;
            }
            DesktopChange::Topology(change) => {
                let measure_set = self.apply_topology_change(change, instance_manager)?;
                return Ok(ChangeOutput::new(measure_set, self.user_state.clone()));
            }
            DesktopChange::ForwardEvents(transitions) => {
                let commands = self.forward_event_transitions(transitions, instance_manager)?;
                let mut changes = Changes::default();
                for command in commands {
                    changes += self.plan(command, scene)?;
                }
                // TODO: Support Into for Changes -> ChangeOutput (after that thing with the
                // UserState is solved)
                return Ok(ChangeOutput::changes(changes));
            }
            DesktopChange::IntegrateInstanceSubmission(instance_id, instance_submission) => {
                let measures = self.apply_instance_submission(
                    instance_id,
                    instance_submission,
                    scene,
                    instance_manager,
                )?;
                return Ok(ChangeOutput::new(measures, self.user_state.clone()));
            }
            DesktopChange::Project(project_change) => {
                self.apply_project_change(project_change, scene)?;
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
            TopologyChange::Add { what, under } => {
                self.aggregates.hierarchy.add(under.clone(), what)?;
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
                // Bug: This should remove target from focus (but how, focus parent or unfocus completely)
                self.unfocus_pointer_if_path_contains(&target, instance_manager)?;
                Ok(self.remove_target(&target)?)
            }
        }
    }

    fn apply_project_change(&mut self, change: ProjectChange, scene: &Scene) -> Result<()> {
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
            ProjectChange::AddLauncher {
                project,
                id,
                profile,
                placement,
            } => {
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
                    placement,
                    profile,
                    massive_geometry::Size::default(),
                    scene,
                    &mut self.fonts.lock(),
                );
                self.aggregates.launchers.insert(id, presenter)?;
            }
            ProjectChange::RemoveLauncher(launch_profile_id) => {
                self.aggregates.launchers.remove(&launch_profile_id)?;
            }
            ProjectChange::SetStartupProfile(launch_profile_id) => {
                self.aggregates.startup_profile = launch_profile_id;
            }
        }

        Ok(())
    }

    /// Apply a command, recursively applying any follow-up commands it causes, and return the
    /// combined [`CommandOutcome`].
    ///
    /// Currently, `UserState` is available through modification `&mut self`, but we simplify a lot
    /// by threading it through and committing it outside of this function.
    pub(super) fn apply_command(
        &mut self,
        command: DesktopCommand,
        scene: &Scene,
        instance_manager: &mut InstanceManager,
    ) -> Result<ChangeOutput> {
        // warn!("Apply command: {command:?}");

        let user_state = if command.resets_zoom() {
            UserState::Focused
        } else {
            self.user_state.clone()
        };

        match command {
            DesktopCommand::StartInstance { .. } | DesktopCommand::StopInstance(..) => {
                unreachable!();
            }

            DesktopCommand::Project(project_command) => {
                let measure_set = self.apply_project_command(project_command, scene)?;
                Ok(ChangeOutput::new(measure_set, user_state))
            }

            DesktopCommand::ZoomIn => {
                let user_state = self
                    .event_router
                    .focused()
                    .map(|focused| {
                        zoom_in(
                            &self.aggregates.hierarchy,
                            &self.aggregates.launchers,
                            focused.clone(),
                            user_state.clone(),
                        )
                    })
                    .unwrap_or(user_state);
                Ok(ChangeOutput::measureless(user_state))
            }

            DesktopCommand::ZoomOut => {
                let user_state = self
                    .event_router
                    .focused()
                    .map(|focused| {
                        zoom_out(
                            &self.aggregates.hierarchy,
                            &self.aggregates.launchers,
                            focused.clone(),
                            user_state.clone(),
                        )
                    })
                    .unwrap_or(user_state);
                Ok(ChangeOutput::measureless(user_state))
            }

            DesktopCommand::Navigate(direction) => {
                let user_state =
                    self.apply_navigate_command(direction, instance_manager, user_state)?;
                Ok(ChangeOutput::measureless(user_state))
            }
        }
    }

    fn apply_instance_submission(
        &mut self,
        instance: InstanceId,
        submission: InstanceSubmission,
        scene: &Scene,
        instance_manager: &InstanceManager,
    ) -> Result<MeasureSet> {
        let (changes, pacing) = submission.into_parts();
        let mut measures = MeasureSet::Empty;

        for change in changes.release() {
            measures += self.apply_instance_change(instance, change, scene, instance_manager)?;
        }

        self.set_instance_pacing(instance, pacing);
        Ok(measures)
    }

    fn apply_instance_change(
        &mut self,
        instance: InstanceId,
        change: InstanceChange,
        scene: &Scene,
        instance_manager: &InstanceManager,
    ) -> Result<MeasureSet> {
        match change {
            InstanceChange::Scene(change) => {
                scene.push_change(change);
                Ok(MeasureSet::Empty)
            }
            InstanceChange::CreateView(creation_info) => {
                let measure_set = self.present_view(instance, &creation_info, scene)?;

                // If this instance is currently focused and the new view is primary, make it
                // foreground so that the view is focused.
                if let (Some(DesktopTarget::Instance(focused_instance)), ViewRole::Primary) =
                    (self.event_router.focused(), &creation_info.role)
                    && *focused_instance == instance
                {
                    // TODO: Don't call this directly, this should go through the change application.
                    self.focus(
                        &DesktopTarget::View(creation_info.id),
                        instance_manager,
                        FocusReason::PromotePrimaryView,
                    )?;
                }

                Ok(measure_set)
            }
            InstanceChange::DestroyView(id) => {
                let view_path: ViewPath = (instance, id).into();
                self.hide_view(view_path)
            }
            InstanceChange::View(view_id, command) => {
                let view_path: ViewPath = (instance, view_id).into();
                self.apply_view_change(view_path, command)?;
                Ok(MeasureSet::Empty)
            }
            // This makes sure that all pending Scene Changes from the Instance have been collected
            // before we drop the last ref the instance has to its parent location (which in turn
            // may push other deletes to the Scene).
            InstanceChange::End(_) => Ok(MeasureSet::Empty),
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
}
