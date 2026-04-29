use anyhow::{Result, bail};
use log::warn;

use massive_applications::{InstanceId, ViewCreationInfo};
use massive_shell::Scene;

use super::DesktopTarget;
use crate::instance_manager::ViewPath;
use crate::instance_presenter::InstancePresenter;
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

        let initial_center_translation =
            originating_presenter.map(|op| op.layout_transform_animation.value().translate);

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
        scene: &Scene,
    ) -> Result<()> {
        let Some(instance_presenter) = self.aggregates.instances.get_mut(&instance) else {
            bail!("Instance not found");
        };

        instance_presenter.present_view(view_creation_info, scene)?;

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

        instance_presenter.hide_view(path.view)?;

        // Robustness: What about focus?

        // And remove the view.
        self.remove_target(&DesktopTarget::View(path.view))?;

        Ok(())
    }
}
