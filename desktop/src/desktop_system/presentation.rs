use anyhow::{Result, bail};
use log::warn;

use massive_applications::{InstanceId, ViewCreationInfo};
use massive_geometry::Vector3;
use massive_shell::Scene;

use super::DesktopTarget;
use crate::desktop_system::effects::MeasureSet;
use crate::instance_manager::ViewPath;
use crate::instance_presenter::{InstancePresenter, InstanceRoot};
use crate::projects::LaunchProfileId;

use super::DesktopSystem;

#[derive(Debug)]
pub struct OriginationDetails {
    pub insertion_pos: usize,
    pub initial_center_translation: Option<Vector3>,
}

impl DesktopSystem {
    pub(super) fn present_instance(
        &mut self,
        launcher: LaunchProfileId,
        initial_center_translation: Option<Vector3>,
        instance: InstanceId,
        root: InstanceRoot,
        scene: &Scene,
    ) -> Result<()> {
        let (render_instance_background, launcher_location) = {
            let launcher = self
                .aggregates
                .launchers
                .get(&launcher)
                .expect("Launcher not found");
            (
                launcher.should_render_instance_background(),
                launcher.location(),
            )
        };

        let presenter = InstancePresenter::new(
            initial_center_translation,
            render_instance_background,
            root,
            launcher_location,
            scene,
        );

        self.aggregates.instances.insert(instance, presenter)?;

        // Architecture: This should be a kind of rule applied implicitly.
        // Inform the launcher to fade out.
        self.aggregates
            .launchers
            .get_mut(&launcher)
            .expect("Launcher not found")
            .fade_out();

        Ok(())
    }

    pub fn get_origination_details(
        &self,
        launcher: LaunchProfileId,
        originator: InstanceId,
    ) -> OriginationDetails {
        let originating_presenter = self.aggregates.instances.get(&originator);

        let initial_center_translation =
            originating_presenter.map(|op| op.layout_transform_animation.latest_value().translate);

        let nested = self.aggregates.hierarchy.get_nested(&launcher.into());

        let insertion_pos = nested
            .iter()
            .position(|i| *i == DesktopTarget::Instance(originator))
            .map(|i| i + 1)
            .unwrap_or(nested.len());

        OriginationDetails {
            insertion_pos,
            initial_center_translation,
        }
    }

    pub(super) fn hide_instance(&mut self, instance: InstanceId) -> Result<MeasureSet> {
        let Some(DesktopTarget::Launcher(launcher)) =
            self.aggregates.hierarchy.parent(&instance.into()).cloned()
        else {
            bail!("Internal error: Launcher not found");
        };

        let effects = self.remove_target(&DesktopTarget::Instance(instance))?;
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

        Ok(effects)
    }

    pub(super) fn present_view(
        &mut self,
        instance: InstanceId,
        view_creation_info: &ViewCreationInfo,
        scene: &Scene,
    ) -> Result<MeasureSet> {
        let Some(instance_presenter) = self.aggregates.instances.get_mut(&instance) else {
            bail!("Instance not found (present_view)");
        };

        instance_presenter.present_view(view_creation_info, scene)?;

        // Add the view to the hierarchy.
        self.aggregates.hierarchy.add(
            DesktopTarget::Instance(instance),
            DesktopTarget::View(view_creation_info.id),
        )?;

        Ok(DesktopTarget::Instance(instance).into())
    }

    pub(super) fn hide_view(&mut self, path: ViewPath) -> Result<MeasureSet> {
        let Some(instance_presenter) = self.aggregates.instances.get_mut(&path.instance) else {
            warn!("Can't hide view: Instance for view not found");
            // Robustness: Decide if this should return an error.
            return Ok(MeasureSet::Empty);
        };

        instance_presenter.hide_view(path.view)?;

        // Robustness: What about focus?

        // And remove the view.
        self.remove_target(&DesktopTarget::View(path.view))
    }
}
