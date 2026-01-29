use std::{collections::HashMap, time::Duration};

use anyhow::{Result, bail};

use massive_applications::{InstanceId, ViewCreationInfo, ViewEvent, ViewId, ViewRole};
use massive_geometry::{RectPx, SizePx};
use massive_layout::{self as layout, LayoutAxis};
use massive_scene::Transform;
use massive_shell::Scene;

use crate::{
    instance_presenter::{InstancePresenter, InstancePresenterState, PrimaryViewPresenter},
    navigation,
    navigation::NavigationNode,
};

#[derive(Debug, Default)]
/// Manages the presentation of a horizontal band of instances.
pub struct BandPresenter {
    // Robustness don't make these pub.
    pub instances: HashMap<InstanceId, InstancePresenter>,
    /// The Instances in order as they take up space in a final configuration. Exiting
    /// instances are not anymore in this list.
    pub ordered: Vec<InstanceId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BandTarget {
    Instance(InstanceId),
    View(ViewId),
}

impl BandPresenter {
    pub const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);

    pub fn is_empty(&self) -> bool {
        self.ordered.is_empty()
    }

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
            creation_info: view_creation_info.clone(),
        };

        let presenter = InstancePresenter {
            state: InstancePresenterState::Presenting {
                view: view_presenter,
            },
            panel_size: view_creation_info.size(),
            rect: RectPx::zero(),
            center_animation: scene.animated(Default::default()),
        };

        self.instances.insert(instance, presenter);
        self.ordered.push(instance);

        Ok(())
    }

    /// Present an instance originating from another.
    ///
    /// The originating is used for two purposes.
    /// - For determining the panel size.
    /// - For determining where to insert the new instance in the band (default is right next to
    ///   originating).
    pub fn present_instance(
        &mut self,
        instance: InstanceId,
        originating_from: Option<InstanceId>,
        default_panel_size: SizePx,
        scene: &Scene,
    ) -> Result<()> {
        let originating_presenter =
            originating_from.and_then(|originating_from| self.instances.get(&originating_from));

        let presenter = InstancePresenter {
            state: InstancePresenterState::Appearing,
            panel_size: originating_presenter
                .map(|p| p.panel_size)
                .unwrap_or(default_panel_size),
            rect: RectPx::zero(),
            // Correctness: We animate from 0,0 if no originating exist. Need a position here.
            center_animation: scene.animated(
                originating_presenter
                    .map(|op| op.center_animation.value())
                    .unwrap_or_default(),
            ),
        };

        if self.instances.insert(instance, presenter).is_some() {
            bail!("Instance already presented");
        }

        let pos = self
            .ordered
            .iter()
            .position(|i| Some(*i) == originating_from)
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

        match &presenter.state {
            InstancePresenterState::Presenting { view } => {
                let view = PrimaryViewPresenter {
                    creation_info: view.creation_info.clone(),
                };
                presenter.state = InstancePresenterState::Disappearing { view };
            }
            InstancePresenterState::Disappearing { .. } => {
                bail!("Instance is already disappearing")
            }
            InstancePresenterState::Appearing => {
                bail!("Cannot hide instance that is still appearing")
            }
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

        let Some(instance_presenter) = self.instances.get_mut(&instance) else {
            bail!("Instance not found");
        };

        if !matches!(instance_presenter.state, InstancePresenterState::Appearing) {
            bail!("Primary view is already presenting");
        }

        // Feature: Add a alpha animation just for the view.
        instance_presenter.panel_size = view_creation_info.size();
        instance_presenter.state = InstancePresenterState::Presenting {
            view: PrimaryViewPresenter {
                creation_info: view_creation_info.clone(),
            },
        };

        Ok(())
    }

    pub fn hide_view(&mut self, _id: ViewId) -> Result<()> {
        bail!("Hiding views is not supported yet");
    }

    pub fn layout(&self) -> layout::Layout<InstanceId, 2> {
        let mut band_builder = layout::container(None, LayoutAxis::HORIZONTAL);

        for instance_id in &self.ordered {
            let presenter = &self.instances[instance_id];
            band_builder.child(layout::leaf(*instance_id, presenter.panel_size));
        }

        band_builder.layout()
    }

    pub fn set_instance_rect(&mut self, instance_id: InstanceId, rect: RectPx, animate: bool) {
        self.instances
            .get_mut(&instance_id)
            .expect("Internal error: Instance not found")
            .set_rect(rect, animate);
    }

    pub fn navigation(&self) -> NavigationNode<'_, BandTarget> {
        navigation::container(None, || {
            let mut nodes = Vec::new();

            for instance_id in &self.ordered {
                let presenter = &self.instances[instance_id];
                let instance_nav = presenter
                    .navigation()
                    .map_target(BandTarget::View)
                    .with_target(BandTarget::Instance(*instance_id));
                nodes.push(instance_nav);
            }

            nodes
        })
    }

    /// Process an event directly targeted at the band itself (i.e. its border / title)
    pub fn process(&self, _view_event: ViewEvent) -> Result<()> {
        Ok(())
    }

    pub fn apply_animations(&self) {
        self.instances.values().for_each(|p| p.apply_animations());
    }

    /// Return the primary's view's (final) transform.
    ///
    /// It's view might not yet visible.
    ///
    /// `None` if the instance does not exist.
    pub fn instance_transform(&self, instance: InstanceId) -> Option<Transform> {
        self.instances
            .get(&instance)
            .map(|instance| instance.center_animation.final_value().into())
    }
}
