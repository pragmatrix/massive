use std::time::Duration;

use anyhow::{Result, bail};
use massive_animation::{Animated, Interpolation};
use massive_applications::{ViewCreationInfo, ViewId, ViewRole};
use massive_geometry::{Color, Point, Rect, SizePx, Transform, Vector3};
use massive_scene::{At, Handle, Location, Object, ToLocation, Visual};
use massive_shapes::{self as shapes, Shape};
use massive_shell::Scene;

pub const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);
const INSTANCE_BACKGROUND_COLOR: Color = Color::rgb_u32(0x282828);

#[derive(Debug)]
pub struct InstancePresenter {
    state: InstancePresenterState,
    /// The instance layout transform stores the panel center translation and yaw rotation.
    /// Position-only consumers should read `layout_transform_animation.*.translate`.
    pub layout_transform_animation: Animated<Transform>,
    has_meaningful_previous_position: bool,
    background: Option<InstanceBackground>,
}

#[derive(Debug)]
struct InstanceBackground {
    transform: Handle<Transform>,
    visual: Handle<Visual>,
    local_rect: Rect,
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
        let background = show_background.then(|| {
            let transform = Transform::IDENTITY.enter(scene);
            let local_location = transform.to_location().relative_to(location).enter(scene);
            let visual = background_shape(Rect::ZERO).at(local_location).enter(scene);

            InstanceBackground {
                transform,
                visual,
                local_rect: Rect::ZERO,
            }
        });

        Self {
            state: InstancePresenterState::WaitingForPrimaryView,
            layout_transform_animation: scene.animated(Transform::from_translation(
                initial_center_translation.unwrap_or_default(),
            )),
            has_meaningful_previous_position: initial_center_translation.is_some(),
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
            todo!("Only primary views are supported yet");
        }

        if !matches!(self.state, InstancePresenterState::WaitingForPrimaryView) {
            bail!("Primary view is already presenting");
        }

        view_creation_info
            .location
            .update_with_if_changed(|location| {
                location.alpha = 0.0;
            });
        let mut alpha = scene.animated(0.0);
        alpha.animate(1.0, STRUCTURAL_ANIMATION_DURATION, Interpolation::CubicOut);

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
        let snap_layout = !self.has_meaningful_previous_position;

        self.apply_layout(size, layout_transform, animate && !snap_layout);
        self.has_meaningful_previous_position = true;
    }

    fn apply_layout(&mut self, size: SizePx, layout_transform: Transform, animate: bool) {
        if let Some(background) = &mut self.background {
            background.local_rect = Rect::from_size((size.width as f64, size.height as f64));
            background.visual.update_with_if_changed(|visual| {
                let local_rect = background.local_rect;
                visual.shapes = [background_shape(local_rect)].into();
            });
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
            self.apply_animations();
        }
    }

    pub fn apply_animations(&mut self) {
        let layout_transform = self.layout_transform_animation.value();

        if let Some(background) = &self.background {
            let local_center = background.local_rect.center();
            background
                .transform
                .update_if_changed(Self::transform_with_layout(layout_transform, local_center));
        }

        // Feature: Hiding animation.
        let Some(view) = self.state.view_mut() else {
            return;
        };

        // Correct the view's position around its local center.
        // Since the centering uses i32, we preserve snapping behavior from the layouter.
        let size = view.creation_info.size();
        let center = Rect::from_size((size.width as f64, size.height as f64)).center();
        let transform =
            Self::transform_with_layout(layout_transform, Point::new(center.x, center.y));

        view.creation_info
            .location
            .value()
            .transform
            .update_if_changed(transform);

        let alpha = view.alpha.value();
        view.creation_info
            .location
            .update_with_if_changed(|location| {
                location.alpha = alpha;
            });
    }

    pub fn transform_with_layout(layout_transform: Transform, local_center: Point) -> Transform {
        let local_center = Vector3::new(local_center.x, local_center.y, 0.0);
        let origin_translation =
            layout_transform.translate - layout_transform.rotate * local_center;
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
