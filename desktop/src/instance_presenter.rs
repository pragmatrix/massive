use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};

use winit::window::CursorIcon;

use massive_animation::{Animated, AnimationAllocator, AnimationProgress, Interpolation, Movement};
use massive_applications::prelude::*;
use massive_applications::{InstanceParameters, ViewCreationInfo, ViewId, ViewRole};
use massive_geometry::{Color, Rect, SizePx, SizedTransform, Transform, Vector3};
use massive_renderer::RenderPacing;
use massive_scene::Ref;
use massive_scene::prelude::*;
use massive_shapes::{self as shapes, Shape};

use crate::projects::{FullScreenMode, InstanceTitleBarPresenter, InstanceTitleBarSpec};

/// What an instance was started as (ADR 0014). A `Base` instance presents in
/// its launcher's persisted Full Screen Mode; an `Assistant` instance carries
/// its own temporary mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InstanceKind {
    #[default]
    Base,
    Assistant,
}

impl InstanceKind {
    /// The Full Screen Mode a new instance of this kind starts in: an
    /// assistant's temporary mode starts regular; a base instance presents in
    /// the launcher's mode.
    pub fn initial_full_screen_mode(self, launcher_mode: FullScreenMode) -> FullScreenMode {
        match self {
            Self::Base => launcher_mode,
            Self::Assistant => FullScreenMode::Regular,
        }
    }
}

/// The Full Screen Mode state of an instance (ADR 0014), combining the two
/// instance kinds with how each one's mode is determined: a `Base` instance
/// presents in its launcher's persisted mode, an `Assistant` owns a temporary
/// mode of its own. Folding the kinds together keeps a base instance with its
/// own mode unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceFullScreen {
    Base,
    Assistant(FullScreenMode),
}

#[derive(Debug, Clone)]
pub struct InstanceRoot {
    layout_transform: Handle<Transform>,
    layout_location: Handle<Location>,

    // The content origin: The top-left corner of the instance's extent (ADR 0019). The title bar
    // and the view are placed relative to it, with the same coordinates the layout uses.
    presentation_transform: Handle<Transform>,
    presentation_location: Handle<Location>,

    // The view's own placement inside the content: centered, scaled smaller in full-screen mode.
    view_transform: Handle<Transform>,
    view_location: Handle<Location>,
}

impl InstanceRoot {
    pub fn new() -> Self {
        let (layout_transform, layout_location) = identity_location().submit();
        let (presentation_transform, presentation_location) = identity_location()
            .relative_to(layout_location.to_ref())
            .submit();

        let (view_transform, view_location) = identity_location()
            .relative_to(presentation_location.to_ref())
            .submit();

        Self {
            layout_transform,
            layout_location,
            presentation_transform,
            presentation_location,
            view_transform,
            view_location,
        }
    }

    /// The view's parent location.
    pub fn view_parent(&self) -> Ref<Location> {
        self.view_location.to_ref()
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
    /// The instance's Full Screen presentation (ADR 0014).
    full_screen: InstanceFullScreen,
    movement: Movement<InstanceMovement>,
    /// Shared animated instance node for background and view.
    /// This avoids per-child world updates that can drift during animation.
    root: InstanceRoot,
    /// Cached because hover placement needs the synchronous target while movement updates are queued.
    target_transform: Transform,
    has_applied_layout: bool,
    pub pacing: RenderPacing,
    background: Option<InstanceBackground>,
    /// ADR 0019: Exists from construction on, so the instance's extent includes it from its first
    /// commit.
    title_bar: InstanceTitleBarPresenter,
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
    view_size: SizePx,
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
    pub fn new(
        initial_center_translation: Option<Vector3>,
        show_background: bool,
        root: InstanceRoot,
        parameters: InstanceParameters,
        parent: Handle<Location>,
        kind: InstanceKind,
        title_bar: InstanceTitleBarSpec,
    ) -> Self {
        root.layout_location.update_if_changed_with(|location| {
            location.parent = parent.to_ref().into();
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
                .at(&root.view_location)
                .submit();

            InstanceBackground {
                visual,
                local_rect: Rect::ZERO,
            }
        });

        let transform = root.layout_transform();
        let location = root.layout_location.clone();
        let movement = movement(
            InstanceMovement::new(initial_center_translation),
            move |movement, context| {
                movement.apply_animations(context, &transform, &location);
            },
        )
        .mount();

        let full_screen = match kind {
            InstanceKind::Base => InstanceFullScreen::Base,
            InstanceKind::Assistant => InstanceFullScreen::Assistant(FullScreenMode::Regular),
        };

        let title_bar =
            InstanceTitleBarPresenter::new(title_bar, root.presentation_location.clone());

        Self {
            state: InstancePresenterState::WaitingForPrimaryView,
            parameters,
            movement,
            root,
            target_transform: Transform::from_translation(initial_center_translation),
            has_applied_layout: has_initial_center_translation,
            pacing: RenderPacing::default(),
            background,
            title_bar,
            full_screen,
        }
    }

    pub fn parameters(&self) -> &InstanceParameters {
        &self.parameters
    }

    /// The instance's kind (ADR 0014): an assistant carries its own temporary
    /// mode, a base instance presents in its launcher's mode.
    pub fn kind(&self) -> InstanceKind {
        match self.full_screen {
            InstanceFullScreen::Base => InstanceKind::Base,
            InstanceFullScreen::Assistant(_) => InstanceKind::Assistant,
        }
    }

    /// The instance's temporary Full Screen Mode, `None` for a base instance
    /// (a base instance presents in its launcher's mode instead).
    pub fn full_screen_mode(&self) -> Option<FullScreenMode> {
        match self.full_screen {
            InstanceFullScreen::Base => None,
            InstanceFullScreen::Assistant(mode) => Some(mode),
        }
    }

    /// Flips the assistant's temporary Full Screen Mode. Panics for a base
    /// instance, which presents in its launcher's mode and carries none.
    pub fn toggle_full_screen_mode(&mut self) {
        let InstanceFullScreen::Assistant(mode) = self.full_screen else {
            panic!("a toggled instance is an assistant carrying a temporary mode");
        };
        self.full_screen = InstanceFullScreen::Assistant(mode.toggled());
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
            movement.view_alpha.animate_with(
                context,
                1.0,
                STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        });

        let view_size = view_creation_info.size();

        self.state = InstancePresenterState::Presenting {
            view: PrimaryViewPresenter {
                creation_info: view_creation_info.clone(),
                window_state: ViewWindowState::default(),
                view_size,
            },
        };

        if let Some(background) = &mut self.background {
            let rect = Rect::from_size(view_size);
            background.update_rect(rect);
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
        view.window_state.title = title.clone();
        self.title_bar.set_title(&title);
        Ok(())
    }

    pub fn set_view_cursor(&mut self, view_id: ViewId, cursor: CursorIcon) -> Result<()> {
        let view = self.presented_view_mut(view_id)?;
        view.window_state.cursor = cursor;
        Ok(())
    }

    /// This fails if the view is not presented.
    pub fn view_window_state(&self, view_id: ViewId) -> Result<&ViewWindowState> {
        self.presenting_view(view_id).map(|view| &view.window_state)
    }

    pub fn primary_view_id(&self) -> Option<ViewId> {
        Some(self.state.view()?.creation_info.id)
    }

    pub fn set_view_layout(
        &mut self,
        view_id: ViewId,
        layout: SizedTransform,
    ) -> Result<Option<SizePx>> {
        let view = self.presented_view_mut(view_id)?;
        let new_size = SizePx::new(layout.size.width as u32, layout.size.height as u32);
        let resize = (view.view_size != new_size).then_some(new_size);
        view.view_size = new_size;

        self.root.view_transform.update_if_changed(layout.transform);

        if let Some(background) = &mut self.background {
            background.update_rect(layout.rect());
        }

        Ok(resize)
    }

    pub fn title_bar_height(&self) -> u32 {
        self.title_bar.measured_height()
    }

    pub fn set_title_bar_layout(&mut self, layout: SizedTransform) {
        self.title_bar.set_layout(layout);
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
        // The content origin is the top-left corner of the extent, while the layout transform is
        // centered on it.
        self.root
            .presentation_transform
            .update_if_changed(Transform::from_translation(Vector3::new(
                -layout.size.width * 0.5,
                -layout.size.height * 0.5,
                0.0,
            )));

        let (target_visibility_alpha, layout_transform) = if visible {
            (1.0, layout.transform)
        } else {
            // Keep panel x/y pose but pull hidden instances back to baseline depth.
            (0.0, layout.transform.with_z(0.0))
        };
        self.target_transform = layout_transform;

        self.movement.modify(move |movement, context| {
            movement.set_layout(context, layout_transform, target_visibility_alpha);
        });
    }

    fn presenting_view(&self, view_id: ViewId) -> Result<&PrimaryViewPresenter> {
        let Some(view) = self.state.view() else {
            bail!("Instance presenter is not presenting a view.")
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
        self.visibility_alpha.animate_if_changed_with(
            context,
            visibility_alpha,
            STRUCTURAL_ANIMATION_DURATION,
            Interpolation::CubicOut,
        );
        self.layout_transform.animate_if_changed_with(
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
        transform.update_if_changed(*self.layout_transform.proceed_with(progress));
        location.update_if_changed_with(|location| {
            location.alpha = *self.view_alpha.proceed_with(progress)
                * *self.visibility_alpha.proceed_with(progress);
        });
    }
}

impl InstanceBackground {
    fn update_rect(&mut self, rect: Rect) {
        self.local_rect = rect;
        self.visual.update_if_changed_with(|visual| {
            visual.shapes = Self::shapes(self.centered_rect());
        });
    }

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
