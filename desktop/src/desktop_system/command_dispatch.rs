use anyhow::{Result, anyhow};
use log::warn;

use massive_applications::{CreationMode, ViewRole};
use massive_shell::Scene;

use super::{DesktopCommand, DesktopSystem, DesktopTarget};
use crate::focus_path::PathResolver;
use crate::instance_manager::InstanceManager;

impl DesktopSystem {
    // Architecture: The current focus is part of the system, so DesktopInteraction should probably be embedded here.
    pub(super) fn apply_command(
        &mut self,
        command: DesktopCommand,
        scene: &Scene,
        instance_manager: &mut InstanceManager,
    ) -> Result<()> {
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
                    .ok_or(anyhow!("Internal error, application not registered"))?;

                let instance =
                    instance_manager.spawn(application, CreationMode::New(parameters))?;

                // Robustness: Should this be a real, logged event?
                // Architecture: Better to start up the primary directly, so that we can remove the PresentInstance command?
                self.apply_command(
                    DesktopCommand::PresentInstance { launcher, instance },
                    scene,
                    instance_manager,
                )
            }

            DesktopCommand::StopInstance(instance) => {
                // Remove the instance from the focus first.
                //
                // Detail: This causes an unfocus event sent to the instance's view which may
                // unexpected while teardown.

                let target = DesktopTarget::Instance(instance);
                let replacement_focus = self
                    .aggregates
                    .hierarchy
                    .resolve_replacement_focus_for_stopping_instance(
                        self.event_router.focused(),
                        instance,
                    );

                if let Some(replacement_focus) = replacement_focus {
                    self.focus(&replacement_focus, instance_manager)?;
                }

                self.unfocus_pointer_if_path_contains(&target, instance_manager)?;

                // This might fail if StopInstance gets triggered with an instance that ended in
                // itself (shouldn't the instance_manager keep it until we finally free it).
                if let Err(e) = instance_manager.request_shutdown(instance) {
                    warn!("Failed to shutdown instance, it may be gone already: {e}");
                };

                // We hide the instance as soon we request a shutdown so that they can't be in the
                // navigation tree anymore.
                self.hide_instance(instance)?;

                Ok(())
            }

            DesktopCommand::PresentInstance { launcher, instance } => {
                let originating_from = self
                    .aggregates
                    .hierarchy
                    .resolve_path(self.event_router.focused())
                    .instance();

                let insertion_index =
                    self.present_instance(launcher, originating_from, instance, scene)?;

                let instance_target = DesktopTarget::Instance(instance);

                // Add this instance to the hierarchy.
                self.aggregates.hierarchy.insert_at(
                    launcher.into(),
                    insertion_index,
                    instance_target.clone(),
                )?;
                self.layouter
                    .mark_reflow_pending(DesktopTarget::Launcher(launcher));

                // Focus it.
                self.focus(&instance_target, instance_manager)?;
                Ok(())
            }

            DesktopCommand::PresentView(instance, creation_info) => {
                self.present_view(instance, &creation_info, scene)?;

                let focused = self.event_router.focused();
                // If this instance is currently focused and the new view is primary, make it
                // foreground so that the view is focused.
                if matches!(focused, Some(DesktopTarget::Instance(i)) if *i == instance)
                    && creation_info.role == ViewRole::Primary
                {
                    self.focus(&DesktopTarget::View(creation_info.id), instance_manager)?;
                }

                Ok(())
            }
            DesktopCommand::HideView(view_path) => self.hide_view(view_path),

            DesktopCommand::Project(project_command) => {
                self.apply_project_command(project_command, scene)
            }

            DesktopCommand::ZoomOut => {
                if let Some(focused) = self.event_router.focused()
                    && let Some(parent) = self.aggregates.hierarchy.parent(focused)
                {
                    self.focus(&parent.clone(), instance_manager)?;
                }
                Ok(())
            }
            DesktopCommand::Navigate(direction) => {
                if let Some(focused) = self.event_router.focused()
                    && let Some(candidate) = self.locate_navigation_candidate(focused, direction)
                {
                    self.focus(&candidate, instance_manager)?;
                }
                Ok(())
            }
        }
    }
}
