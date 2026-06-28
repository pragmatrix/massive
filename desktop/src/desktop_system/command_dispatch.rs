use anyhow::{Context, Result};
use log::{debug, warn};

use massive_applications::{
    CreationMode, InstanceChange, InstanceId, InstanceSubmission, ViewChange, ViewRole,
};
use massive_shell::Scene;

use super::{DesktopCommand, DesktopSystem, DesktopTarget, FocusReason};
use crate::desktop_system::UserState;
use crate::desktop_system::effects::MeasureSet;
use crate::desktop_system::zoom_navigation::{zoom_in, zoom_out};
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::instance_presenter::InstanceRoot;

pub fn combine(
    (ma, _): (MeasureSet, UserState),
    (mb, us): (MeasureSet, UserState),
) -> (MeasureSet, UserState) {
    (ma + mb, us)
}

impl DesktopSystem {
    /// Applies the command and returns a new MeasureSet and UserState.
    ///
    /// Currently, `UserState` is available through modification `&mut self`, but we simplify a lot
    /// by threading it through and committing it outside of this function.
    pub(super) fn apply_command(
        &mut self,
        command: DesktopCommand,
        scene: &Scene,
        instance_manager: &mut InstanceManager,
    ) -> Result<(MeasureSet, UserState)> {
        // warn!("Apply command: {command:?}");

        let user_state = if command.resets_zoom() {
            UserState::Focused
        } else {
            self.user_state.clone()
        };

        match command {
            DesktopCommand::StartInstance {
                launcher,
                parameters,
            } => {
                // Feature: Support starting non-primary applications.
                let application = self
                    .env
                    .applications
                    .get_named(&self.env.primary_application)
                    .context("Internal error, application not registered")?;

                let root = InstanceRoot::new(scene);
                let instance = instance_manager.spawn(
                    application,
                    CreationMode::New(parameters),
                    root.location(),
                )?;

                // Robustness: Should this be a real, logged event?
                // Architecture: Better to start up the primary directly, so that we can remove the PresentInstance command?
                self.apply_command(
                    DesktopCommand::PresentInstance {
                        launcher,
                        instance,
                        root,
                    },
                    scene,
                    instance_manager,
                )
            }

            DesktopCommand::StopInstance(instance) => {
                // Remove the instance from the focus first.
                //
                // Detail: This causes an unfocus event sent to the instance's view which may
                // unexpected while tear down.

                let target = DesktopTarget::Instance(instance);
                let replacement_focus = self
                    .aggregates
                    .hierarchy
                    .resolve_replacement_focus_for_stopping_instance(
                        self.event_router.focused(),
                        instance,
                    );

                if let Some(replacement_focus) = replacement_focus {
                    self.focus(
                        &replacement_focus,
                        instance_manager,
                        FocusReason::StopInstanceReplacement,
                    )?;
                }

                self.unfocus_pointer_if_path_contains(&target, instance_manager)?;

                // This might fail if StopInstance gets triggered with an instance that ended in
                // itself (shouldn't the instance_manager keep it until we finally free it).
                if let Err(e) = instance_manager.request_shutdown(instance) {
                    warn!("Failed to shutdown instance, it may be gone already: {e}");
                };

                // We hide the instance as soon we request a shutdown so that they can't be in the
                // navigation tree anymore.
                let measure_set = self.hide_instance(instance)?;

                Ok((measure_set, user_state))
            }

            DesktopCommand::PresentInstance {
                launcher,
                instance,
                root,
            } => {
                let originating_from = self.focused_path().instance();

                let insertion_index =
                    self.present_instance(launcher, originating_from, instance, root, scene)?;

                let instance_target = DesktopTarget::Instance(instance);

                // Add this instance to the hierarchy.
                self.aggregates.hierarchy.insert_at(
                    launcher.into(),
                    insertion_index,
                    instance_target.clone(),
                )?;

                // Focus it.
                self.focus(
                    &instance_target,
                    instance_manager,
                    FocusReason::PresentInstance,
                )?;
                Ok((MeasureSet::One(launcher.into()), user_state))
            }

            DesktopCommand::IntegrateInstanceSubmission(instance, submission) => {
                let measure_set =
                    self.apply_instance_submission(instance, submission, scene, instance_manager)?;
                Ok((measure_set, user_state))
            }

            DesktopCommand::Project(project_command) => {
                let measure_set = self.apply_project_command(project_command, scene)?;
                Ok((measure_set, user_state))
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
                Ok((MeasureSet::Empty, user_state))
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
                Ok((MeasureSet::Empty, user_state))
            }

            DesktopCommand::NavigateTo(target) => {
                let follow_up_commands =
                    self.navigate_to(target, instance_manager, FocusReason::InputTransition)?;
                let mut r = (MeasureSet::Empty, user_state);
                for command in follow_up_commands {
                    r = combine(r, self.apply_command(command, scene, instance_manager)?);
                }
                Ok(r)
            }
            DesktopCommand::Navigate(direction) => {
                let user_state =
                    self.apply_navigate_command(direction, instance_manager, user_state)?;
                Ok((MeasureSet::Empty, user_state))
            }
        }
    }

    fn apply_instance_submission(
        &mut self,
        instance: InstanceId,
        submission: InstanceSubmission,
        scene: &Scene,
        instance_manager: &mut InstanceManager,
    ) -> Result<MeasureSet> {
        let (changes, pacing) = submission.into_parts();
        let mut effects = MeasureSet::Empty;

        for change in changes.release() {
            effects += self.apply_instance_change(instance, change, scene, instance_manager)?;
        }

        self.set_instance_pacing(instance, pacing);
        Ok(effects)
    }

    fn apply_instance_change(
        &mut self,
        instance: InstanceId,
        change: InstanceChange,
        scene: &Scene,
        instance_manager: &mut InstanceManager,
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
                match (self.event_router.focused(), &creation_info.role) {
                    (Some(DesktopTarget::Instance(focused_instance)), ViewRole::Primary)
                        if *focused_instance == instance =>
                    {
                        self.focus(
                            &DesktopTarget::View(creation_info.id),
                            instance_manager,
                            FocusReason::PromotePrimaryView,
                        )?;
                    }
                    _ => {}
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
