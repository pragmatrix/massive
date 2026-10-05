use anyhow::Result;
use anyhow::bail;
use log::warn;

use massive_applications::{InstanceId, ViewCreationInfo};
use massive_geometry::{Transform, Vector3};
use massive_layout::{Placement, Rect as LayoutRect};

use super::DesktopTarget;
use super::change::{Changes, DesktopChange, InstancePresentation, TopologyChange};
use super::command_dispatch::ChangeOutput;
use super::fullscreen_scale;
use crate::instance_manager::ViewPath;
use crate::instance_presenter::InstancePresenter;
use crate::projects::{FullScreenMode, LaunchProfileId, launcher_mode};

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
        let (render_instance_background, launcher_location) = {
            let launcher = self
                .aggregates
                .launchers
                .get(&launcher_id)
                .expect("Launcher not found");
            let render_instance_background = launcher_mode::renders_instance_background(
                self.aggregates
                    .configuration
                    .launcher(launcher_id)
                    .expect("Launcher not found")
                    .mode,
            );
            (render_instance_background, launcher.location())
        };

        // An assistant owns a temporary Full Screen Mode, starting regular; a
        // base instance is `None` and follows its launcher's mode (ADR 0014).
        let presenter = InstancePresenter::new(
            initial_center_translation,
            render_instance_background,
            root,
            parameters,
            launcher_location,
            kind,
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
    ) -> Result<ChangeOutput> {
        let Some(instance_presenter) = self.aggregates.instances.get_mut(&instance) else {
            bail!("Instance not found (present_view)");
        };

        instance_presenter.present_view(view_creation_info)?;

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

    /// The target the hover outline anchors on for the current focus state.
    ///
    /// While a fullscreen instance is focused, its view covers the whole window,
    /// so the pointer physically sits on the focused view's area whatever it
    /// pointed at before — tracking pointer focus would box the obscured
    /// instance behind the fullscreen one. Keyboard focus is the honest anchor
    /// there (ADR 0014); a view-less fullscreen instance still presents the
    /// fullscreen rect (see [`Self::instance_presentation`]), so the anchor does
    /// not bounce while the view is pending.
    pub(super) fn hover_target(&self) -> Option<&DesktopTarget> {
        let fullscreen_instance_focused = self.focused_path().instance().is_some_and(|instance| {
            self.aggregates.instance_full_screen_mode(instance) == FullScreenMode::FullScreen
        });

        if fullscreen_instance_focused {
            self.event_router.keyboard_focus()
        } else {
            self.event_router
                .pointer_focus()
                .or_else(|| self.event_router.keyboard_focus())
        }
    }

    /// The hover outline's placement for the current focus state.
    pub(super) fn hover_placement(&self) -> Option<Placement<Transform, 2>> {
        let target = self.hover_target()?;
        match target {
            DesktopTarget::Launcher(_) | DesktopTarget::View(_) => Some(self.placement(target)),
            DesktopTarget::Instance(instance) => Some(self.instance_presentation(*instance)),
            _ => None,
        }
    }

    /// The placement an instance presents.
    ///
    /// A fullscreen instance presents window-resolution content at the fullscreen
    /// content scale, at its own center (ADR 0014) — whether or not its view
    /// exists yet. Its raw layout placement is the panel, so anchoring on that
    /// would bounce the hover onto the empty panel for the view-less commit
    /// between `Cmd+T`'s StartInstance and the view's first submission.
    fn instance_presentation(&self, instance: InstanceId) -> Placement<Transform, 2> {
        let placement = self.placement(&DesktopTarget::Instance(instance));
        if self.aggregates.instance_full_screen_mode(instance) != FullScreenMode::FullScreen {
            return placement;
        }

        let content_scale = fullscreen_scale(self.default_panel_size, self.window_state.inner_size);
        let transform = Transform::new(
            placement.transform.translate,
            placement.transform.rotate,
            placement.transform.scale * content_scale,
        );
        let rect = LayoutRect::new(placement.rect.offset, self.window_state.inner_size.into());
        Placement::new(transform, rect)
    }
}
