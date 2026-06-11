use std::{sync::Arc, time::Duration};

use anyhow::{Result, bail};

use massive_animation::{Animated, Interpolation};
use massive_applications::{ViewCreationInfo, ViewId, ViewRole};
use massive_geometry::{Color, Rect, SizePx, Transform, Vector3};
use massive_renderer::RenderPacing;
use massive_scene::{At, Handle, Location, Object, Ref, ToLocation, Visual};
use massive_shapes::{self as shapes, Shape};
use massive_shell::Scene;
use winit::window::CursorIcon;

#[derive(Debug, Clone)]
pub struct InstanceRoot {
    transform: Handle<Transform>,
    location: Handle<Location>,
}

impl InstanceRoot {
    pub fn new(scene: &Scene) -> Self {
        let transform = Transform::IDENTITY.enter(scene);
        let location = transform.to_location().enter(scene);

        Self {
            transform,
            location,
        }
    }

    pub fn location(&self) -> Ref<Location> {
        self.location.to_ref()
    }
}

pub const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);
const INSTANCE_BACKGROUND_COLOR: Color = Color::rgb_u32(0x282828);

#[derive(Debug)]
pub struct InstancePresenter {
    state: InstancePresenterState,
    /// The instance layout transform stores the panel center translation and yaw rotation.
    /// Position-only consumers should read `layout_transform_animation.*.translate`.
    pub layout_transform_animation: Animated<Transform>,
    visibility_alpha: Animated<f32>,
    /// Shared animated instance node for background and view.
    /// This avoids per-child world updates that can drift during animation.
    root: InstanceRoot,
    has_applied_layout: bool,
    pub pacing: RenderPacing,
    background: Option<InstanceBackground>,
}

#[derive(Debug)]
struct InstanceBackground {
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
    window_state: ViewWindowState,
    alpha: Animated<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct ViewWindowState {
    pub title: String,
    pub cursor: CursorIcon,
}

impl InstancePresenter {
    pub fn new(
        initial_center_translation: Option<Vector3>,
        show_background: bool,
        root: InstanceRoot,
        parent: Handle<Location>,
        scene: &Scene,
    ) -> Self {
        root.location.update_if_changed_with(|location| {
            location.parent = Some(parent.to_ref());
        });

        let background = show_background.then(|| {
            let visual = background_shapes(false, Rect::ZERO)
                .at(&root.location)
                .enter(scene);

            InstanceBackground {
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
            visibility_alpha: scene.animated(1.0),
            root,
            has_applied_layout: initial_center_translation.is_some(),
            pacing: RenderPacing::default(),
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

        match self.state {
            InstancePresenterState::WaitingForPrimaryView => {}
            InstancePresenterState::Presenting { .. } | InstancePresenterState::Disappearing => {
                bail!("Primary view is already presenting");
            }
        }

        // Blend in.
        let mut alpha = scene.animated(0.0);
        {
            self.root.location.update_with(|location| {
                location.alpha = 0.0;
            });
            alpha.animate(1.0, STRUCTURAL_ANIMATION_DURATION, Interpolation::CubicOut);
        }

        self.state = InstancePresenterState::Presenting {
            view: PrimaryViewPresenter {
                creation_info: view_creation_info.clone(),
                window_state: ViewWindowState::default(),
                alpha,
            },
        };

        if let Some(background) = &mut self.background {
            background.visual.update_if_changed_with(|visual| {
                visual.location = self.root.location.to_ref();
                visual.shapes = background_shapes(background.visible, background.centered_rect());
            });
        }

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
                // Ignored, we are already disappearing.
                Ok(())
            }
        }
    }

    pub fn set_view_title(&mut self, view_id: ViewId, title: String) -> Result<()> {
        let view = self.presented_view_mut(view_id)?;
        view.window_state.title = title;
        Ok(())
    }

    pub fn set_view_cursor(&mut self, view_id: ViewId, cursor: CursorIcon) -> Result<()> {
        let view = self.presented_view_mut(view_id)?;
        view.window_state.cursor = cursor;
        Ok(())
    }

    pub fn view_window_state(&self, view_id: ViewId) -> Result<&ViewWindowState> {
        self.presented_view(view_id).map(|view| &view.window_state)
    }

    pub fn set_layout(
        &mut self,
        size: SizePx,
        layout_transform: Transform,
        visible: bool,
        animate: bool,
    ) {
        let snap_layout = !self.has_applied_layout;

        self.apply_layout(size, layout_transform, visible, animate && !snap_layout);
        self.has_applied_layout = true;
    }

    fn apply_layout(
        &mut self,
        size: SizePx,
        layout_transform: Transform,
        visible: bool,
        animate: bool,
    ) {
        if let Some(background) = &mut self.background {
            background.local_rect = Rect::from_size((size.width as f64, size.height as f64));
            background.visible = size.width > 0 && size.height > 0;
        }

        let target_visibility_alpha = if visible { 1.0 } else { 0.0 };
        if animate {
            self.visibility_alpha.animate_if_changed(
                target_visibility_alpha,
                STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        } else {
            self.visibility_alpha
                .set_immediately(target_visibility_alpha);
        }

        let layout_transform = if visible {
            layout_transform
        } else {
            // Keep panel x/y pose but pull hidden instances back to baseline depth.
            Transform::new(
                Vector3::new(
                    layout_transform.translate.x,
                    layout_transform.translate.y,
                    0.0,
                ),
                layout_transform.rotate,
                layout_transform.scale,
            )
        };

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

        if let Some(background) = &mut self.background {
            background.visual.update_if_changed_with(|visual| {
                // Background geometry stays in instance space; views apply their own local offset.
                visual.shapes = background_shapes(background.visible, background.centered_rect());
            });
        }

        // Apply transform/alpha animation updates for this frame.
        self.apply_animations();
    }

    pub fn apply_animations(&mut self) {
        let layout_transform = self.layout_transform_animation.value();
        self.root.transform.update_if_changed(*layout_transform);

        let view_alpha = match &mut self.state {
            InstancePresenterState::WaitingForPrimaryView => 1.0,
            InstancePresenterState::Presenting { view } => *view.alpha.value(),
            InstancePresenterState::Disappearing => 0.0,
        };
        let alpha = view_alpha * *self.visibility_alpha.value();
        self.root.location.update_if_changed_with(|location| {
            location.alpha = alpha;
        });
    }

    fn presented_view(&self, view_id: ViewId) -> Result<&PrimaryViewPresenter> {
        let Some(view) = self.state.view() else {
            bail!("A view needs to be updated, but instance presenter is not presenting a view.")
        };

        if view.creation_info.id != view_id {
            bail!("Invalid view: It's not related to anything we present");
        }

        Ok(view)
    }

    fn presented_view_mut(&mut self, view_id: ViewId) -> Result<&mut PrimaryViewPresenter> {
        let Some(view) = self.state.view_mut() else {
            bail!("A view needs to be updated, but instance presenter is not presenting a view.")
        };

        if view.creation_info.id != view_id {
            bail!("Invalid view: It's not related to anything we present");
        }

        Ok(view)
    }
}

impl InstanceBackground {
    fn centered_rect(&self) -> Rect {
        self.local_rect - self.local_rect.center()
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
