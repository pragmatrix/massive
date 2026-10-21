use anyhow::Result;
use anyhow::bail;
use log::warn;

use massive_applications::{InstanceId, ViewCreationInfo};
use massive_geometry::{Transform, Vector3};
use massive_layout::Placement;

use super::change::{Changes, DesktopChange, InstancePresentation, TopologyChange};
use super::command_dispatch::ChangeOutput;
use super::{DesktopTarget, FocusableTarget};
use crate::instance_manager::ViewPath;
use crate::instance_presenter::InstancePresenter;
use crate::projects::{LaunchProfileId, launcher_mode};

use super::DesktopSystem;

#[derive(Debug)]
pub struct OriginationDetails {
    pub insertion_pos: usize,
    pub initial_center_translation: Option<Vector3>,
}

impl DesktopSystem {
    pub(super) fn present_instance(&mut self, presentation: InstancePresentation) -> Result<()> {
        let InstancePresentation {
            launcher: launcher_id,
            initial_center_translation,
            instance,
            root,
            parameters,
            kind,
        } = presentation;
        let (render_instance_background, launcher_location, launcher_name) = {
            let launcher = self
                .aggregates
                .launchers
                .get(&launcher_id)
                .expect("Launcher not found");
            let launcher_configuration = self
                .aggregates
                .configuration
                .launcher(launcher_id)
                .expect("Launcher not found");
            let render_instance_background =
                launcher_mode::renders_instance_background(launcher_configuration.mode);
            (
                render_instance_background,
                launcher.location(),
                launcher_configuration.name.clone(),
            )
        };

        // An assistant owns a temporary Full Screen Mode, starting regular; a
        // primary instance is `None` and follows its launcher's mode (ADR 0014).
        let presenter = InstancePresenter::new(
            instance,
            initial_center_translation,
            render_instance_background,
            root,
            parameters,
            launcher_location,
            kind,
            &launcher_name,
            self.aggregates.instance_title_bar_metrics,
        );

        self.aggregates.instances.insert(instance, presenter)?;

        // Architecture: This should be a kind of rule applied implicitly.
        // Inform the launcher to fade out.
        self.aggregates
            .launchers
            .get_mut(&launcher_id)
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
            originating_presenter.map(|presenter| presenter.latest_transform().translate);

        let nested = self.aggregates.hierarchy.get_nested(&launcher.into());

        let insertion_pos = nested
            .iter()
            .position(|i| *i == originator.into())
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

    pub fn present_view(
        &mut self,
        instance: InstanceId,
        view_creation_info: &ViewCreationInfo,
    ) -> Result<ChangeOutput> {
        let Some(instance_presenter) = self.aggregates.instances.get_mut(&instance) else {
            bail!("Instance not found (present_view)");
        };

        instance_presenter.present_view(view_creation_info)?;

        // Add the view to the hierarchy as a separate topology change.
        let changes: Changes = DesktopChange::Topology(TopologyChange::Add {
            what: DesktopTarget::view(instance, view_creation_info.id),
            under: instance.into(),
            after: None,
        })
        .into();

        Ok(ChangeOutput::changes(changes))
    }

    pub fn hide_view(&mut self, path: ViewPath) -> Result<ChangeOutput> {
        let Some(instance_presenter) = self.aggregates.instances.get_mut(&path.instance) else {
            warn!("Can't hide view: Instance for view not found");
            // Robustness: Decide if this should return an error.
            return Ok(ChangeOutput::default());
        };

        instance_presenter.hide_view(path.view)?;

        // Remove the view from the hierarchy as a separate topology change. The remove change
        // also retargets focus away from the removed subtree.
        let changes: Changes = DesktopChange::Topology(TopologyChange::Remove(
            DesktopTarget::view(path.instance, path.view),
        ))
        .into();

        Ok(ChangeOutput::changes(changes))
    }

    /// The hover outline follows pointer focus, or keyboard focus during keyboard navigation. A
    /// selection beyond the focused slot widens the keyboard focus's outline to the selection
    /// (ADR 0025).
    pub fn hover_placement(&self) -> Option<Placement<Transform, 2>> {
        if self.event_router.pointer_focus().is_none()
            && let Some(focused) = self.keyboard_focus.hover_outline_focus()
            && let Some(placement) = self.selection.outline_placement(
                focused,
                &self.layout_state,
                &self.aggregates.layout_context(
                    self.default_panel_size,
                    self.window_state.inner_size,
                    self.focused_path().instance(),
                ),
            )
        {
            return Some(placement);
        }
        let target = self.hover_target()?;
        match &target {
            DesktopTarget::Project(..)
            | DesktopTarget::Launcher(_)
            | DesktopTarget::Instance(..) => Some(self.placement(&target)),
            DesktopTarget::Desktop => None,
        }
    }

    fn hover_target(&self) -> Option<DesktopTarget> {
        let target = self.event_router.pointer_focus().cloned().or_else(|| {
            self.keyboard_focus
                .hover_outline_focus()
                .map(DesktopTarget::from)
        })?;

        // The outline marks what a click selects; a view outlines its whole instance (ADR 0019).
        Some(match target.selected_target() {
            FocusableTarget::View(instance, _) => instance.into(),
            target => target.into(),
        })
    }
}
