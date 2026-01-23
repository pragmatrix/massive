use std::{collections::HashMap, time::Duration};

use anyhow::{Result, bail};
use derive_more::From;

use massive_applications::{InstanceId, ViewCreationInfo, ViewId, ViewRole};
use massive_geometry::{PointPx, RectPx};
use massive_layout::{Box, LayoutAxis};
use massive_scene::Transform;
use massive_shell::Scene;

use crate::instance_presenter::{InstancePresenter, InstancePresenterState, PrimaryViewPresenter};

#[derive(Debug, Clone, Copy, From)]
enum LayoutId {
    Root,
    Instance(InstanceId),
}

type Layouter<'a> = massive_layout::Layouter<'a, LayoutId, 2>;

#[derive(Debug, Default)]
/// Manages the presentation of a horizontal band of instances.
pub struct BandPresenter {
    instances: HashMap<InstanceId, InstancePresenter>,
    /// The Instances in order as they take up space in a final configuration. Exiting
    /// instances are not anymore in this list.
    ordered: Vec<InstanceId>,
}

impl BandPresenter {
    pub const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);
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
            state: InstancePresenterState::Presenting {
                view: view_presenter,
            },
            panel_size: view_creation_info.size(),
            center_animation: scene.animated(Default::default()),
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
            center_animation: scene.animated(originating_presenter.center_animation.value()),
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

        match &presenter.state {
            InstancePresenterState::Presenting { view } => {
                let view = PrimaryViewPresenter {
                    view: view.view.clone(),
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
                view: view_creation_info.clone(),
            },
        };

        Ok(())
    }

    pub fn hide_view(&mut self, _id: ViewId) -> Result<()> {
        bail!("Hiding views is not supported yet");
    }

    /// Compute the current layout and animate the views to their positions.
    pub fn layout(&mut self, animate: bool) {
        let mut layout = Layouter::root(LayoutId::Root, LayoutAxis::HORIZONTAL);

        for instance_id in &self.ordered {
            let presenter = &self.instances[instance_id];
            layout.leaf((*instance_id).into(), presenter.panel_size);
        }

        layout.place_inline(PointPx::origin(), |(id, rect)| {
            if let LayoutId::Instance(instance_id) = id {
                self.set_instance_rect(instance_id, box_to_rect(rect), animate);
            }
        });
    }

    fn set_instance_rect(&mut self, instance_id: InstanceId, rect: RectPx, animate: bool) {
        self.instances
            .get_mut(&instance_id)
            .expect("Internal error: Instance not found")
            .set_rect(rect, animate);
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

fn box_to_rect(([x, y], [w, h]): Box<2>) -> RectPx {
    RectPx::new((x, y).into(), (w as i32, h as i32).into())
}
