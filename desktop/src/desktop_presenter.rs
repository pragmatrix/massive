use std::{collections::HashMap, time::Duration};

use anyhow::{Result, bail};

use massive_animation::{Animated, Interpolation};
use massive_applications::{InstanceId, ViewCreationInfo, ViewId, ViewRole};
use massive_geometry::{RectPx, SizePx, Vector3};
use massive_layout::{LayoutInfo, LayoutNode, layout};
use massive_scene::Transform;
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
    pub const INSTANCE_TRANSITION_DURATION: Duration = Duration::from_millis(500);
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
            panel_size: view_creation_info.size(),
            center_animation: scene.animated(Default::default()),
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
            center_animation: scene.animated(originating_presenter.center_animation.value()),
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

        let Some(instance_presenter) = self.instances.get_mut(&instance) else {
            bail!("Instance not found");
        };

        if instance_presenter.state != InstancePresenterState::Appearing {
            bail!("Primary view is already presenting");
        }

        // Feature: Add a alpha animation just for the view.
        instance_presenter.panel_size = view_creation_info.size();
        instance_presenter.view = Some(PrimaryViewPresenter {
            view: view_creation_info.clone(),
        });
        instance_presenter.state = InstancePresenterState::Presenting;

        Ok(())
    }

    pub fn hide_view(&mut self, _id: ViewId) -> Result<()> {
        bail!("Hiding views is not supported yet");
    }

    /// Compute the current layout and animate the views to their positions.
    pub fn layout(&mut self, animate: bool) {
        layout(self, &mut LayoutContext { animate });
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

#[derive(Debug)]
struct InstancePresenter {
    state: InstancePresenterState,
    // The size of the panel. Including borders.
    panel_size: SizePx,
    /// The center of the instance's panel. This is also the point the camera should look at if its
    /// at rest.
    center_animation: Animated<Vector3>,
    // The primary view inside the panel.
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

impl InstancePresenter {
    pub fn apply_animations(&self) {
        let Some(view) = &self.view else {
            return;
        };

        // Get the translation for the instance.
        let mut translation = self.center_animation.value();

        // And correct the view's position.
        // Since the centering uses i32, we snap to pixel here (what we want!).
        let center = view.view.extents.center().to_f64();
        translation -= Vector3::new(center.x, center.y, 0.0);

        view.view
            .location
            .value()
            .transform
            .update_if_changed(translation.into());
    }
}

// layout

#[derive(Debug)]
struct LayoutContext {
    animate: bool,
}

impl LayoutNode<LayoutContext> for DesktopPresenter {
    type Rect = RectPx;

    fn layout_info(&self, _context: &LayoutContext) -> LayoutInfo<SizePx> {
        LayoutInfo::container(self.ordered.len())
    }

    fn get_child_mut(
        &mut self,
        index: usize,
    ) -> &mut dyn LayoutNode<LayoutContext, Rect = Self::Rect> {
        let instance = self.ordered[index];
        self.instances
            .get_mut(&instance)
            .expect("Internal error: Order table does not match the instance map")
    }
}

impl LayoutNode<LayoutContext> for InstancePresenter {
    type Rect = RectPx;

    fn layout_info(&self, _context: &LayoutContext) -> LayoutInfo<SizePx> {
        self.panel_size.into()
    }

    fn set_rect(&mut self, rect: Self::Rect, context: &mut LayoutContext) {
        let translation = (rect.origin.x as f64, rect.origin.y as f64, 0.0).into();

        if context.animate {
            self.center_animation.animate_if_changed(
                translation,
                DesktopPresenter::INSTANCE_TRANSITION_DURATION,
                Interpolation::CubicOut,
            );
        } else {
            self.center_animation.set_immediately(translation);
            self.apply_animations();
        }
    }
}
