use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};

use winit::window::CursorIcon;

use massive_animation::{
    Animated, AnimationAllocator, AnimationProgress, Interpolation, Movement, MovementRuntime,
};
use massive_applications::{InstanceParameters, ViewCreationInfo, ViewId, ViewRole};
use massive_geometry::{Color, Rect, Size, SizePx, SizedTransform, Transform, Vector3};
use massive_renderer::RenderPacing;
use massive_scene::{
    At, Handle, Location, Object, Ref, StageIdentityLocation, ToLocationRelative, Visual,
};
use massive_shapes::{self as shapes, Shape};
use massive_shell::Scene;

#[derive(Debug, Clone)]
pub struct InstanceRoot {
    layout_transform: Handle<Transform>,
    layout_location: Handle<Location>,
    presentation_transform: Handle<Transform>,
    presentation_location: Handle<Location>,
}

impl InstanceRoot {
    pub fn new(scene: &Scene) -> Self {
        let (layout_transform, layout_location) = scene.enter_identity_location();
        let presentation_transform = Transform::IDENTITY.enter(scene);
        let presentation_location = presentation_transform
            .to_location_relative(layout_location.to_ref())
            .enter(scene);

        Self {
            layout_transform,
            layout_location,
            presentation_transform,
            presentation_location,
        }
    }

    pub fn location(&self) -> Ref<Location> {
        self.presentation_location.to_ref()
    }

    fn layout_transform(&self) -> Handle<Transform> {
        self.layout_transform.clone()
    }
}

pub const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);
const INSTANCE_BACKGROUND_COLOR: Color = Color::rgb_u32(0x282828);

#[derive(Debug)]
pub struct InstancePresenter {
    state: InstancePresenterState,
    parameters: InstanceParameters,
    movement: Movement<InstanceMovement>,
    /// Shared animated instance node for background and view.
    /// This avoids per-child world updates that can drift during animation.
    root: InstanceRoot,
    /// Cached because hover placement needs the synchronous target while movement updates are queued.
    target_transform: Transform,
    regular_size: SizePx,
    has_applied_layout: bool,
    pub pacing: RenderPacing,
    background: Option<InstanceBackground>,
}

#[derive(Debug)]
struct InstanceBackground {
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
    window_state: ViewWindowState,
    presentation: ViewPresentation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ViewPresentation {
    #[default]
    Regular,
    FullScreen(SizePx),
}

#[derive(Debug)]
struct InstanceMovement {
    /// The instance layout transform stores the panel center translation and yaw rotation.
    layout_transform: Animated<Transform>,
    visibility_alpha: Animated<f32>,
    view_alpha: Animated<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct ViewWindowState {
    pub title: String,
    pub cursor: CursorIcon,
}

impl InstancePresenter {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        initial_center_translation: Option<Vector3>,
        regular_size: SizePx,
        show_background: bool,
        root: InstanceRoot,
        parameters: InstanceParameters,
        parent: Handle<Location>,
        scene: &Scene,
        movement_runtime: &mut MovementRuntime,
    ) -> Self {
        root.layout_location.update_if_changed_with(|location| {
            location.parent = Some(parent.to_ref());
        });

        let has_initial_center_translation = initial_center_translation.is_some();
        let initial_center_translation = initial_center_translation.unwrap_or_default();
        root.layout_transform
            .update_if_changed(Transform::from_translation(initial_center_translation));
        root.layout_location.update_if_changed_with(|location| {
            location.alpha = 0.0;
        });

        let background = show_background.then(|| {
            let visual = InstanceBackground::shapes(Rect::ZERO)
                .at(&root.presentation_location)
                .enter(scene);

            InstanceBackground {
                visual,
                local_rect: Rect::ZERO,
            }
        });

        let transform = root.layout_transform();
        let location = root.layout_location.clone();
        let movement = movement_runtime
            .movement(
                InstanceMovement::new(initial_center_translation),
                move |movement, context| {
                    movement.apply_animations(context, &transform, &location);
                },
            )
            .mount();

        Self {
            state: InstancePresenterState::WaitingForPrimaryView,
            parameters,
            movement,
            root,
            target_transform: Transform::from_translation(initial_center_translation),
            regular_size,
            has_applied_layout: has_initial_center_translation,
            pacing: RenderPacing::default(),
            background,
        }
    }

    pub fn presents_primary_view(&self) -> bool {
        self.state.view().is_some()
    }

    pub fn parameters(&self) -> &InstanceParameters {
        &self.parameters
    }

    pub fn latest_transform(&self) -> Transform {
        *self.root.layout_transform.value()
    }

    pub fn present_view(&mut self, view_creation_info: &ViewCreationInfo) -> Result<()> {
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

        // Architecture: I don't think we should modify alpha here, may be nest another location
        // below it?
        self.root.layout_location.update_with(|location| {
            location.alpha = 0.0;
        });
        self.movement.modify(move |movement, context| {
            // Same here, this looks weird.
            movement.view_alpha.snap(0.0);
            movement.view_alpha.animate(
                context,
                1.0,
                STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        });

        self.state = InstancePresenterState::Presenting {
            view: PrimaryViewPresenter {
                creation_info: view_creation_info.clone(),
                window_state: ViewWindowState::default(),
                presentation: ViewPresentation::Regular,
            },
        };

        self.apply_view_presentation();

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

    pub fn primary_view_id(&self) -> Option<ViewId> {
        Some(self.state.view()?.creation_info.id)
    }

    pub fn set_full_screen(&mut self, size: SizePx) -> Option<ViewId> {
        let view = self.state.view_mut()?;
        if view.presentation == ViewPresentation::FullScreen(size) {
            return None;
        }

        view.presentation = ViewPresentation::FullScreen(size);
        let view_id = view.creation_info.id;
        self.apply_view_presentation();
        Some(view_id)
    }

    pub fn set_regular(&mut self) -> Option<ViewId> {
        let view = self.state.view_mut()?;
        if view.presentation == ViewPresentation::Regular {
            return None;
        }

        view.presentation = ViewPresentation::Regular;
        let view_id = view.creation_info.id;
        self.apply_view_presentation();
        Some(view_id)
    }

    pub fn presentation_size(&self) -> SizePx {
        match self.state.view().map(|view| view.presentation) {
            Some(ViewPresentation::FullScreen(size)) => size,
            Some(ViewPresentation::Regular) | None => self.regular_size,
        }
    }

    pub fn full_screen_layout_size(&self) -> Option<Size> {
        let ViewPresentation::FullScreen(size) = self.state.view()?.presentation else {
            return None;
        };
        let scale = self.root.presentation_transform.value().scale;
        Some(Size::new(
            size.width as f64 * scale,
            size.height as f64 * scale,
        ))
    }

    pub fn set_layout(&mut self, layout: SizedTransform, visible: bool, animate: bool) {
        let snap_layout = !self.has_applied_layout || !animate;

        self.apply_layout(layout, visible);
        if snap_layout {
            self.movement.snap();
        }
        self.has_applied_layout = true;
    }

    fn apply_layout(&mut self, layout: SizedTransform, visible: bool) {
        let (target_visibility_alpha, layout_transform) = if visible {
            (1.0, layout.transform)
        } else {
            // Keep panel x/y pose but pull hidden instances back to baseline depth.
            (0.0, layout.transform.with_z(0.0))
        };
        self.target_transform = layout_transform;
        self.regular_size = (layout.size.width as u32, layout.size.height as u32).into();

        self.movement.modify(move |movement, context| {
            movement.set_layout(context, layout_transform, target_visibility_alpha);
        });

        self.apply_view_presentation();
    }

    fn apply_view_presentation(&mut self) {
        let size = self.presentation_size();
        let scale = match self.state.view().map(|view| view.presentation) {
            Some(ViewPresentation::FullScreen(_)) if size.width > 0 && size.height > 0 => {
                (self.regular_size.width as f64 / size.width as f64)
                    .min(self.regular_size.height as f64 / size.height as f64)
            }
            Some(ViewPresentation::FullScreen(_)) | Some(ViewPresentation::Regular) | None => 1.0,
        };
        self.root
            .presentation_transform
            .update_if_changed(Transform::from_scale(scale));

        if let Some(background) = &mut self.background {
            background.local_rect = Size::new(size.width as f64, size.height as f64).to_rect();
            background.visual.update_if_changed_with(|visual| {
                // Background geometry stays in instance space; views apply their own local offset.
                visual.shapes = InstanceBackground::shapes(background.centered_rect());
            });
        }
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

impl InstanceMovement {
    fn new(initial_center_translation: Vector3) -> Self {
        Self {
            layout_transform: Transform::from_translation(initial_center_translation).into(),
            visibility_alpha: 1.0.into(),
            view_alpha: 0.0.into(),
        }
    }

    fn set_layout(
        &mut self,
        context: &mut dyn AnimationAllocator,
        layout_transform: Transform,
        visibility_alpha: f32,
    ) {
        self.visibility_alpha.animate_if_changed(
            context,
            visibility_alpha,
            STRUCTURAL_ANIMATION_DURATION,
            Interpolation::CubicOut,
        );
        self.layout_transform.animate_if_changed(
            context,
            layout_transform,
            STRUCTURAL_ANIMATION_DURATION,
            Interpolation::CubicOut,
        );
    }

    fn apply_animations(
        &mut self,
        progress: AnimationProgress,
        transform: &Handle<Transform>,
        location: &Handle<Location>,
    ) {
        // Apply transform and alpha animation updates for this frame.
        transform.update_if_changed(*self.layout_transform.proceed(progress));
        location.update_if_changed_with(|location| {
            location.alpha =
                *self.view_alpha.proceed(progress) * *self.visibility_alpha.proceed(progress);
        });
    }
}

impl InstanceBackground {
    fn centered_rect(&self) -> Rect {
        self.local_rect - self.local_rect.center()
    }

    fn shapes(rect: Rect) -> Arc<[Shape]> {
        (!rect.is_empty())
            .then(|| background_shape(rect))
            .into_iter()
            .collect()
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
