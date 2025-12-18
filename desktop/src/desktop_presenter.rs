use std::{collections::HashMap, time::Duration};

use anyhow::{Result, bail};

use massive_animation::{Animated, Interpolation};
use massive_applications::{InstanceId, ViewCreationInfo, ViewId, ViewRole};
use massive_geometry::{Signed, SizePx, Vector3};
use massive_shell::Scene;

#[derive(Debug, Default)]
/// Manages the presentation of the desktop's user interface.
pub struct DesktopPresenter {
    instances: HashMap<InstanceId, InstancePresenter>,
    /// The Instances in order as they take up space in a final configuration. Exiting
    /// instances are not anymore in this list.
    ordered: Vec<InstanceId>,
}

impl DesktopPresenter {
    /// Present the primary instance and its primary role view.
    ///
    /// For now this can not be done by separately presenting an instance and a view because we
    /// don't support creating an instance with an undefined panel size.
    ///
    /// This is also only possible if there are no other instances yet present.
    pub fn present_primary_instance(
        &mut self,
        instance: InstanceId,
        view_creation_info: &ViewCreationInfo,
        scene: &Scene,
    ) -> Result<()> {
        if !self.instances.is_empty() {
            bail!("Primary instance is already presenting");
        }

        let view_presenter = PrimaryViewPresenter {
            view: view_creation_info.clone(),
        };

        let presenter = InstancePresenter {
            state: InstancePresenterState::Appearing,
            panel_size: view_creation_info.size,
            translation_animation: scene.animated(Default::default()),
            view: Some(view_presenter),
        };

        self.instances.insert(instance, presenter);
        self.ordered.push(instance);

        Ok(())
    }

    /// Present an instance originating from another.
    pub fn present_instance(
        &mut self,
        instance: InstanceId,
        originating_from: InstanceId,
        scene: &Scene,
    ) -> Result<()> {
        let Some(originating_presenter) = self.instances.get(&originating_from) else {
            bail!("Originating presenter does not exist");
        };

        let presenter = InstancePresenter {
            state: InstancePresenterState::Appearing,
            panel_size: originating_presenter.panel_size,
            translation_animation: scene
                .animated(originating_presenter.translation_animation.value()),
            view: None,
        };

        if self.instances.insert(instance, presenter).is_some() {
            bail!("Instance already presented");
        }

        let pos = self
            .ordered
            .iter()
            .position(|i| *i == originating_from)
            .map(|i| i + 1)
            .unwrap_or(self.ordered.len());

        // Even though it's not yet visible, make place for it.
        self.ordered.insert(pos, instance);

        Ok(())
    }

    #[allow(unused)]
    pub fn hide_instance(&mut self, instance: InstanceId) -> Result<()> {
        let Some(presenter) = self.instances.get_mut(&instance) else {
            bail!("Instance not found");
        };

        if presenter.state != InstancePresenterState::Disappearing {
            presenter.state = InstancePresenterState::Disappearing;
        } else {
            bail!("Instance is already disappearing")
        }

        self.ordered.retain(|i| *i != instance);

        Ok(())
    }

    pub fn present_view(
        &mut self,
        instance: InstanceId,
        view_creation_info: &ViewCreationInfo,
    ) -> Result<()> {
        if view_creation_info.role != ViewRole::Primary {
            todo!("Only primary views are supported yet");
        }

        let Some(presenter) = self.instances.get_mut(&instance) else {
            bail!("Instance not found");
        };

        if presenter.state != InstancePresenterState::Appearing {
            bail!("Primary view is already presenting");
        }

        // Feature: Add a alpha animation just for the view.
        presenter.panel_size = view_creation_info.size;
        presenter.view = Some(PrimaryViewPresenter {
            view: view_creation_info.clone(),
        });
        presenter.state = InstancePresenterState::Presenting;

        Ok(())
    }

    pub fn hide_view(&mut self, _id: ViewId) -> Result<()> {
        bail!("Hiding views is not supported yet");
    }

    /// Compute the current layout and animate the views to their positions.
    pub fn layout(&mut self, animate: bool) {
        let mut max_panel_size = SizePx::zero();

        for instance in &self.ordered {
            let instance = &self.instances[instance];
            max_panel_size = max_panel_size.max(instance.panel_size);
        }

        for (i, instance) in self.ordered.iter().enumerate() {
            let translation = ((max_panel_size.width as i32 * i as i32) as f64, 0.0, 0.0);

            let instance = self
                .instances
                .get_mut(instance)
                .expect("Internal error: Instance does not exist");

            if animate {
                instance.translation_animation.animate_if_changed(
                    translation.into(),
                    Duration::from_millis(500),
                    Interpolation::CubicOut,
                );
            } else {
                instance
                    .translation_animation
                    .set_immediately(translation.into());
            }
        }
    }

    pub fn apply_animations(&self) {
        for presenter in self.instances.values() {
            if let Some(view) = &presenter.view {
                let translation = presenter.translation_animation.value();
                view.view
                    .location
                    .value()
                    .transform
                    .update_if_changed(translation.into());
            }
        }
    }
}

#[derive(Debug)]
struct InstancePresenter {
    state: InstancePresenterState,
    // The size of the panel. Including borders.
    panel_size: SizePx,
    translation_animation: Animated<Vector3>,
    // The view inside the panel.
    view: Option<PrimaryViewPresenter>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum InstancePresenterState {
    /// No view yet, or just appearing, animating in.
    Appearing,
    Presenting,
    Disappearing,
}

#[derive(Debug)]
struct PrimaryViewPresenter {
    view: ViewCreationInfo,
}
