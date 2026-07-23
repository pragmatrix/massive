use anyhow::{Result, bail};
use log::warn;

use massive_applications::{InstanceId, InstanceParameters, ViewCreationInfo};
use massive_geometry::{Point, Transform, Vector3};
use massive_layout::{Placement, Size as LayoutSize};
use massive_shell::Scene;

use super::DesktopTarget;
use crate::desktop_system::change::{Changes, DesktopChange, TopologyChange};
use crate::desktop_system::command_dispatch::ChangeOutput;
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
        parameters: InstanceParameters,
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
            parameters,
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
            originating_presenter.map(|op| op.layout_transform_animation.latest().translate);

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

    pub fn hide_instance(&mut self, launcher: LaunchProfileId, instance: InstanceId) -> Result<()> {
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
    ) -> Result<ChangeOutput> {
        let Some(instance_presenter) = self.aggregates.instances.get_mut(&instance) else {
            bail!("Instance not found (present_view)");
        };

        instance_presenter.present_view(view_creation_info, scene)?;

        // Add the view to the hierarchy as a separate topology change.
        let changes: Changes = DesktopChange::Topology(TopologyChange::Add {
            what: DesktopTarget::View(view_creation_info.id),
            under: DesktopTarget::Instance(instance),
            after: None,
        })
        .into();

        Ok(ChangeOutput::changes(changes))
    }

    pub(super) fn hide_view(&mut self, path: ViewPath) -> Result<ChangeOutput> {
        let Some(instance_presenter) = self.aggregates.instances.get_mut(&path.instance) else {
            warn!("Can't hide view: Instance for view not found");
            // Robustness: Decide if this should return an error.
            return Ok(ChangeOutput::default());
        };

        instance_presenter.hide_view(path.view)?;

        // Remove the view from the hierarchy as a separate topology change. The remove change
        // also retargets focus away from the removed subtree.
        let changes: Changes =
            DesktopChange::Topology(TopologyChange::Remove(DesktopTarget::View(path.view))).into();

        Ok(ChangeOutput::changes(changes))
    }

    pub(super) fn sync_hover_with_target(&mut self, target: Option<&DesktopTarget>) {
        let Some(target) = target else {
            self.desktop_presenter.set_hover_placement(None);
            return;
        };

        let hover_placement = match target {
            target @ (DesktopTarget::Instance(..) | DesktopTarget::View(..)) => self
                .aggregates
                .hierarchy
                .instance_of_target(target)
                .map(|instance_id| self.instance_hover_placement(instance_id)),
            DesktopTarget::Launcher(launcher_id) => {
                Some(self.placement(&DesktopTarget::Launcher(*launcher_id)))
            }
            _ => None,
        };

        self.desktop_presenter.set_hover_placement(hover_placement);
    }

    fn instance_hover_placement(&self, instance_id: InstanceId) -> Placement<Transform, 2> {
        let mut placement = self.placement(&DesktopTarget::Instance(instance_id));

        let instance_presenter = self
            .aggregates
            .instances
            .get(&instance_id)
            .expect("Instance not found");
        let launcher_id = self.aggregates.hierarchy.launcher_of_instance(instance_id);
        let launcher_placement = self.placement(&DesktopTarget::Launcher(launcher_id));

        // Keep hover aligned with animated instance motion by composing the current instance-local
        // animated transform with the launcher's world transform.
        placement.transform = Transform::compose_with_anchor(
            launcher_placement.transform,
            layout_center(launcher_placement.rect.size),
            *instance_presenter.layout_transform_animation.latest(),
            layout_center(placement.rect.size),
        );

        placement
    }
}

fn layout_center(size: LayoutSize<2>) -> Point {
    Point::new(size[0] as f64 * 0.5, size[1] as f64 * 0.5)
}
