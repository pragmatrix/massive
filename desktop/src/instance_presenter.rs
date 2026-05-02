use std::{sync::Arc, time::Duration};

use anyhow::{Result, bail};
use massive_animation::{Animated, Interpolation};
use massive_applications::{ViewCreationInfo, ViewId, ViewRole};
use massive_geometry::{Color, Point, Rect, SizePx, Transform, Vector3};
use massive_scene::{At, Handle, Location, Object, ToLocation, Visual};
use massive_shapes::{self as shapes, Shape};
use massive_shell::Scene;

pub const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);
const INSTANCE_BACKGROUND_COLOR: Color = Color::rgb_u32(0x282828);
const INSTANCE_BACKGROUND_LOCAL_Z_OFFSET: f64 = -1.0;

#[derive(Debug)]
pub struct InstancePresenter {
    state: InstancePresenterState,
    /// The instance layout transform stores the panel center translation and yaw rotation.
    /// Position-only consumers should read `layout_transform_animation.*.translate`.
    pub layout_transform_animation: Animated<Transform>,
    /// Shared animated instance node for background and view.
    /// This avoids per-child world updates that can drift during animation.
    instance_transform: Handle<Transform>,
    instance_location: Handle<Location>,
    has_applied_layout: bool,
    background: Option<InstanceBackground>,
}

#[derive(Debug)]
struct InstanceBackground {
    transform: Handle<Transform>,
    visual: Handle<Visual>,
    local_rect: Rect,
    visible: bool,
}

#[derive(Debug)]
enum InstancePresenterState {
    /// No view yet, animating in.
    WaitingForPrimaryView,
    Presenting {
        view: PrimaryViewPresenter,
    },
    Disappearing,
}

#[derive(Debug)]
struct PrimaryViewPresenter {
    creation_info: ViewCreationInfo,
    alpha: Animated<f32>,
}

impl InstancePresenter {
    pub fn new(
        initial_center_translation: Option<Vector3>,
        show_background: bool,
        location: Handle<Location>,
        scene: &Scene,
    ) -> Self {
        let instance_transform = Transform::IDENTITY.enter(scene);
        let instance_location = instance_transform
            .to_location()
            .relative_to(&location)
            .enter(scene);

        let background = show_background.then(|| {
            let transform = Transform::IDENTITY.enter(scene);
            let local_location = transform
                .to_location()
                .relative_to(&instance_location)
                .enter(scene);
            let visual = background_shapes(false, Rect::ZERO)
                .at(local_location)
                .enter(scene);

            InstanceBackground {
                transform,
                visual,
                local_rect: Rect::ZERO,
                visible: false,
            }
        });

        Self {
            state: InstancePresenterState::WaitingForPrimaryView,
            layout_transform_animation: scene.animated(Transform::from_translation(
                initial_center_translation.unwrap_or_default(),
            )),
            instance_transform,
            instance_location,
            has_applied_layout: initial_center_translation.is_some(),
            background,
        }
    }

    pub fn presents_primary_view(&self) -> bool {
        self.state.view().is_some()
    }

    pub fn present_view(
        &mut self,
        view_creation_info: &ViewCreationInfo,
        scene: &Scene,
    ) -> Result<()> {
        if view_creation_info.role != ViewRole::Primary {
            bail!("Only primary views are supported yet");
        }

        if !matches!(self.state, InstancePresenterState::WaitingForPrimaryView) {
            bail!("Primary view is already presenting");
        }

        // Blend in.
        let mut alpha = scene.animated(0.0);
        {
            view_creation_info
                .location
                .update_with_if_changed(|location| {
                    location.parent = Some(self.instance_location.clone());
                    location.alpha = 0.0;
                });
            alpha.animate(1.0, STRUCTURAL_ANIMATION_DURATION, Interpolation::CubicOut);
        }

        self.state = InstancePresenterState::Presenting {
            view: PrimaryViewPresenter {
                creation_info: view_creation_info.clone(),
                alpha,
            },
        };

        Ok(())
    }

    pub fn hide_view(&mut self, view_id: ViewId) -> Result<()> {
        match &self.state {
            InstancePresenterState::WaitingForPrimaryView => {
                bail!(
                    "A view needs to be hidden, but instance presenter waits for a view with a primary role."
                )
            }
            InstancePresenterState::Presenting { view } => {
                if view.creation_info.id == view_id {
                    // Feature: this should initiate a disappearing animation?
                    self.state = InstancePresenterState::Disappearing;
                    Ok(())
                } else {
                    bail!("Invalid view: It's not related to anything we present");
                }
            }
            InstancePresenterState::Disappearing => {
                // ignored, we are already disappearing.
                Ok(())
            }
        }
    }

    pub fn set_layout(&mut self, size: SizePx, layout_transform: Transform, animate: bool) {
        let snap_layout = !self.has_applied_layout;

        self.apply_layout(size, layout_transform, animate && !snap_layout);
        self.has_applied_layout = true;
    }

    fn apply_layout(&mut self, size: SizePx, layout_transform: Transform, animate: bool) {
        if let Some(background) = &mut self.background {
            background.local_rect = Rect::from_size((size.width as f64, size.height as f64));
            background.visible = size.width > 0 && size.height > 0;
        }

        if animate {
            self.layout_transform_animation.animate_if_changed(
                layout_transform,
                STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        } else {
            self.layout_transform_animation
                .set_immediately(layout_transform);
        }

        self.apply_animations();

        if let Some(background) = &mut self.background {
            background.visual.update_with_if_changed(|visual| {
                visual.shapes = background_shapes(background.visible, background.local_rect);
            });
        }
    }

    pub fn apply_animations(&mut self) {
        let layout_transform = self.layout_transform_animation.value();
        self.instance_transform.update_if_changed(layout_transform);

        if let Some(background) = &self.background {
            let background_center = background.local_rect.center();
            background
                .transform
                .update_if_changed(Self::child_transform_with_local_z_offset(
                    background_center,
                    INSTANCE_BACKGROUND_LOCAL_Z_OFFSET,
                ));
        }

        // Feature: Hiding animation.
        let Some(view) = self.state.view_mut() else {
            return;
        };

        // Keep i32 midpoint snapping for view-local center to preserve previous alignment behavior.
        let size = view.creation_info.size();
        let view_center = Point::new((size.width / 2) as f64, (size.height / 2) as f64);
        let transform = Self::child_transform_with_local_z_offset(view_center, 0.0);
        let location = &view.creation_info.location;

        location.value().transform.update_if_changed(transform);

        let alpha = view.alpha.value();
        location.update_with_if_changed(|location| {
            location.alpha = alpha;
        });
    }

    pub fn transform_with_layout(layout_transform: Transform, local_center: Point) -> Transform {
        Self::transform_with_layout_and_local_z_offset(layout_transform, local_center, 0.0)
    }

    fn child_transform_with_local_z_offset(local_center: Point, local_z_offset: f64) -> Transform {
        Transform::from_translation(Vector3::new(
            -local_center.x,
            -local_center.y,
            local_z_offset,
        ))
    }

    fn transform_with_layout_and_local_z_offset(
        layout_transform: Transform,
        local_center: Point,
        local_z_offset: f64,
    ) -> Transform {
        let local_center = Vector3::new(local_center.x, local_center.y, 0.0);
        let local_z_offset = Vector3::new(0.0, 0.0, local_z_offset);
        let origin_translation =
            layout_transform.translate + layout_transform.rotate * (local_z_offset - local_center);
        Transform::new(
            origin_translation,
            layout_transform.rotate,
            layout_transform.scale,
        )
    }
}

impl InstancePresenterState {
    fn view(&self) -> Option<&PrimaryViewPresenter> {
        match self {
            Self::WaitingForPrimaryView => None,
            Self::Presenting { view } => Some(view),
            Self::Disappearing => None,
        }
    }

    fn view_mut(&mut self) -> Option<&mut PrimaryViewPresenter> {
        match self {
            Self::WaitingForPrimaryView => None,
            Self::Presenting { view } => Some(view),
            Self::Disappearing => None,
        }
    }
}

fn background_shape(rect: Rect) -> Shape {
    shapes::Rect::new(rect, INSTANCE_BACKGROUND_COLOR).into()
}

fn background_shapes(visible: bool, rect: Rect) -> Arc<[Shape]> {
    visible
        .then(|| background_shape(rect))
        .into_iter()
        .collect()
}
