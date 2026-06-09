use anyhow::{Context, Result};
use log::{debug, warn};

use massive_applications::{
    CreationMode, InstanceChange, InstanceId, InstanceSubmission, ViewChange, ViewRole,
};
use massive_shell::Scene;

use super::effects::DesktopEffect;
use super::effects::Effects;
use super::{DesktopCommand, DesktopSystem, DesktopTarget};
use crate::focus_path::PathResolver;
use crate::instance_manager::{InstanceManager, ViewPath};
use crate::instance_presenter::InstanceRoot;

impl DesktopSystem {
    // Architecture: The current focus is part of the system, so DesktopInteraction should probably be embedded here.
    pub(super) fn apply_command(
        &mut self,
        command: DesktopCommand,
        scene: &Scene,
        instance_manager: &mut InstanceManager,
    ) -> Result<Effects> {
        // warn!("Apply command: {command:?}");
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

                let mut effects = Effects::None;

                if let Some(replacement_focus) = replacement_focus {
                    effects += self.focus(&replacement_focus, instance_manager)?;
                }

                effects += self.unfocus_pointer_if_path_contains(&target, instance_manager)?;

                // This might fail if StopInstance gets triggered with an instance that ended in
                // itself (shouldn't the instance_manager keep it until we finally free it).
                if let Err(e) = instance_manager.request_shutdown(instance) {
                    warn!("Failed to shutdown instance, it may be gone already: {e}");
                };

                // We hide the instance as soon we request a shutdown so that they can't be in the
                // navigation tree anymore.
                effects += self.hide_instance(instance)?;

                Ok(effects)
            }

            DesktopCommand::PresentInstance {
                launcher,
                instance,
                root,
            } => {
                let originating_from = self
                    .aggregates
                    .hierarchy
                    .resolve_path(self.event_router.focused())
                    .instance();

                let insertion_index =
                    self.present_instance(launcher, originating_from, instance, root, scene)?;

                let instance_target = DesktopTarget::Instance(instance);

                // Add this instance to the hierarchy.
                self.aggregates.hierarchy.insert_at(
                    launcher.into(),
                    insertion_index,
                    instance_target.clone(),
                )?;
                let mut effects =
                    Effects::from(DesktopEffect::Measure(DesktopTarget::Launcher(launcher)));

                // Focus it.
                effects += self.focus(&instance_target, instance_manager)?;
                Ok(effects)
            }

            DesktopCommand::IntegrateInstanceSubmission(instance, submission) => {
                self.apply_instance_submission(instance, submission, scene, instance_manager)
            }

            DesktopCommand::Project(project_command) => {
                self.apply_project_command(project_command, scene)
            }

            DesktopCommand::ZoomOut => {
                if let Some(focused) = self.event_router.focused()
                    && let Some(parent) = self.aggregates.hierarchy.parent(focused)
                {
                    return self.focus(&parent.clone(), instance_manager);
                }
                Ok(Effects::None)
            }
            DesktopCommand::Navigate(direction) => {
                if let Some(focused) = self.event_router.focused()
                    && let Some(candidate) = self.locate_navigation_candidate(focused, direction)
                {
                    return self.focus(&candidate, instance_manager);
                }
                Ok(Effects::None)
            }
        }
    }

    fn apply_instance_submission(
        &mut self,
        instance: InstanceId,
        submission: InstanceSubmission,
        scene: &Scene,
        instance_manager: &mut InstanceManager,
    ) -> Result<Effects> {
        let (changes, pacing) = submission.into_parts();
        let mut effects = Effects::None;

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
    ) -> Result<Effects> {
        match change {
            InstanceChange::Scene(change) => {
                scene.push_change(change);
                Ok(Effects::None)
            }
            InstanceChange::CreateView(creation_info) => {
                let mut effects = self.present_view(instance, &creation_info, scene)?;

                // If this instance is currently focused and the new view is primary, make it
                // foreground so that the view is focused.
                match (self.event_router.focused(), &creation_info.role) {
                    (Some(DesktopTarget::Instance(focused_instance)), ViewRole::Primary)
                        if *focused_instance == instance =>
                    {
                        effects +=
                            self.focus(&DesktopTarget::View(creation_info.id), instance_manager)?;
                    }
                    _ => {}
                }

                Ok(effects)
            }
            InstanceChange::DestroyView(id) => {
                let view_path: ViewPath = (instance, id).into();
                self.hide_view(view_path)
            }
            InstanceChange::View(view_id, command) => {
                let view_path: ViewPath = (instance, view_id).into();
                self.apply_view_change(view_path, command)
            }
            // This makes sure that all pending Scene Changes from the Instance have been collected
            // before we drop the last ref the instance has to its parent location (which in turn
            // may push other deletes to the Scene).
            InstanceChange::End(_) => Ok(Effects::None),
        }
    }

    fn apply_view_change(&mut self, view_path: ViewPath, change: ViewChange) -> Result<Effects> {
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

        Ok(Effects::None)
    }
}
