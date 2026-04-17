use anyhow::{Result, bail};
use log::warn;

use massive_applications::{InstanceId, ViewCreationInfo, ViewRole};
use massive_shell::Scene;

use super::DesktopTarget;
use crate::instance_manager::ViewPath;
use crate::instance_presenter::{InstancePresenter, InstancePresenterState, PrimaryViewPresenter};
use crate::projects::LaunchProfileId;

use super::DesktopSystem;

impl DesktopSystem {
    pub(super) fn present_instance(
        &mut self,
        launcher: LaunchProfileId,
        originating_from: Option<InstanceId>,
        instance: InstanceId,
        scene: &Scene,
    ) -> Result<usize> {
        let originating_presenter = originating_from
            .and_then(|originating_from| self.aggregates.instances.get(&originating_from));

        let background_for_instance = self
            .aggregates
            .launchers
            .get(&launcher)
            .expect("Launcher not found")
            .should_render_instance_background();

        // Correctness: We animate from 0,0 if no originating exist. Need a position here.
        let initial_center_translation = originating_presenter
            .map(|op| op.layout_transform_animation.value().translate)
            .unwrap_or_default();

        let presenter = InstancePresenter::new(
            initial_center_translation,
            background_for_instance,
            self.aggregates.project_presenter.location.clone(),
            scene,
        );

        self.aggregates.instances.insert(instance, presenter)?;

        let nested = self.aggregates.hierarchy.get_nested(&launcher.into());
        let insertion_pos = if let Some(originating_from) = originating_from {
            nested
                .iter()
                .position(|i| *i == DesktopTarget::Instance(originating_from))
                .map(|i| i + 1)
                .unwrap_or(nested.len())
        } else {
            0
        };

        // Inform the launcher to fade out.
        self.aggregates
            .launchers
            .get_mut(&launcher)
            .expect("Launcher not found")
            .fade_out();

        Ok(insertion_pos)
    }

    pub(super) fn hide_instance(&mut self, instance: InstanceId) -> Result<()> {
        let Some(DesktopTarget::Launcher(launcher)) =
            self.aggregates.hierarchy.parent(&instance.into()).cloned()
        else {
            bail!("Internal error: Launcher not found");
        };

        self.remove_target(&DesktopTarget::Instance(instance))?;
        self.aggregates.instances.remove(&instance)?;

        if !self
            .aggregates
            .hierarchy
            .entry(&launcher.into())
            .has_nested()
        {
            self.aggregates
                .launchers
                .get_mut(&launcher)
                .expect("Launcher not found")
                .fade_in();
        }

        Ok(())
    }

    pub(super) fn present_view(
        &mut self,
        instance: InstanceId,
        view_creation_info: &ViewCreationInfo,
    ) -> Result<()> {
        if view_creation_info.role != ViewRole::Primary {
            todo!("Only primary views are supported yet");
        }

        let Some(instance_presenter) = self.aggregates.instances.get_mut(&instance) else {
            bail!("Instance not found");
        };

        if !matches!(
            instance_presenter.state,
            InstancePresenterState::WaitingForPrimaryView
        ) {
            bail!("Primary view is already presenting");
        }

        // Architecture: Move this transition in the InstancePresenter
        //
        // Feature: Add a alpha animation just for the view.
        instance_presenter.state = InstancePresenterState::Presenting {
            view: PrimaryViewPresenter {
                creation_info: view_creation_info.clone(),
            },
        };

        // Add the view to the hierarchy.
        self.aggregates.hierarchy.add(
            DesktopTarget::Instance(instance),
            DesktopTarget::View(view_creation_info.id),
        )?;
        self.layouter
            .mark_reflow_pending(DesktopTarget::Instance(instance));

        Ok(())
    }

    pub(super) fn hide_view(&mut self, path: ViewPath) -> Result<()> {
        let Some(instance_presenter) = self.aggregates.instances.get_mut(&path.instance) else {
            warn!("Can't hide view: Instance for view not found");
            // Robustness: Decide if this should return an error.
            return Ok(());
        };

        // Architecture: Move this into the InstancePresenter (don't make state pub).
        match &instance_presenter.state {
            InstancePresenterState::WaitingForPrimaryView => {
                bail!(
                    "A view needs to be hidden, but instance presenter waits for a view with a primary role."
                )
            }
            InstancePresenterState::Presenting { view } => {
                if view.creation_info.id == path.view {
                    // Feature: this should initiate a disappearing animation?
                    instance_presenter.state = InstancePresenterState::Disappearing;
                } else {
                    bail!("Invalid view: It's not related to anything we present");
                }
            }
            InstancePresenterState::Disappearing => {
                // ignored, we are already disappearing.
            }
        }

        // Robustness: What about focus?

        // And remove the view.
        self.remove_target(&DesktopTarget::View(path.view))?;

        Ok(())
    }
}