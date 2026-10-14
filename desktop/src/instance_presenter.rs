use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use uuid::Uuid;
use winit::keyboard::ModifiersState;
use winit::window::CursorIcon;

use massive_animation::{Animated, AnimationAllocator, AnimationProgress, Interpolation, Movement};
use massive_applications::prelude::*;
use massive_applications::{InstanceId, InstanceParameters, ViewCreationInfo, ViewId, ViewRole};
use massive_geometry::{Color, Quaternion, Rect, SizePx, SizedTransform, Transform, Vector3};
use massive_layout::{
    ChildPlacement, LayoutAxis, MeasuredLayout, Offset, Placement, Size as LayoutSize,
};
use massive_renderer::RenderPacing;
use massive_scene::Ref;
use massive_scene::prelude::*;
use massive_shapes::{self as shapes, Shape};

use crate::desktop_system::change::Zoom;
use crate::desktop_system::{
    AppliedLayout, DesktopCommand, InstancePart, KeyContext, KeyHandler, KeyInput, KeyOutcome,
    LayoutContext, Shortcut, TargetLayout, fullscreen_scale, place_container_children, view_size,
};
use crate::projects::{FullScreenMode, TitleBarPresenter, TitleBarStyle};
use crate::view_presenter::ViewPresenter;

/// What an instance was started as (ADR 0014). A `Primary` instance presents in
/// its launcher's persisted Full Screen Mode; an `Assistant` instance carries
/// its own temporary mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InstanceKind {
    #[default]
    Primary,
    Assistant,
}

impl InstanceKind {
    /// Shift starts an assistant (ADR 0014).
    pub fn from_modifiers(modifiers: ModifiersState) -> Self {
        if modifiers.shift_key() {
            Self::Assistant
        } else {
            Self::Primary
        }
    }

    /// The Full Screen Mode a new instance of this kind starts in: an
    /// assistant's temporary mode starts regular; a primary instance presents in
    /// the launcher's mode.
    pub fn initial_full_screen_mode(self, launcher_mode: FullScreenMode) -> FullScreenMode {
        match self {
            Self::Primary => launcher_mode,
            Self::Assistant => FullScreenMode::Regular,
        }
    }
}

/// The Full Screen Mode state of an instance (ADR 0014), combining the two
/// instance kinds with how each one's mode is determined: a `Primary` instance
/// presents in its launcher's persisted mode, an `Assistant` owns a temporary
/// mode of its own. Folding the kinds together keeps a primary instance with its
/// own mode unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceFullScreen {
    Primary,
    Assistant(FullScreenMode),
}

#[derive(Debug, Clone)]
pub struct InstanceRoot {
    layout_transform: Handle<Transform>,
    layout_location: Handle<Location>,

    // The content origin: The top-left corner of the instance's extent (ADR 0019). The title bar
    // and the view are placed relative to it, with the same coordinates the layout uses.
    content_transform: Handle<Transform>,
    content_location: Handle<Location>,

    // The view's own placement inside the content: centered, scaled smaller in full-screen mode.
    view_transform: Handle<Transform>,
    view_location: Handle<Location>,
}

impl InstanceRoot {
    pub fn new() -> Self {
        let (layout_transform, layout_location) = identity_location().submit();
        let (content_transform, content_location) = identity_location()
            .relative_to(layout_location.to_ref())
            .submit();

        let (view_transform, view_location) = identity_location()
            .relative_to(content_location.to_ref())
            .submit();

        Self {
            layout_transform,
            layout_location,
            content_transform,
            content_location,
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
/// A view change arriving before the primary view appears or after the instance began to
/// disappear.
const NOT_PRESENTING: &str = "Instance presenter is not presenting a view";

#[derive(Debug)]
pub struct InstancePresenter {
    id: InstanceId,
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
    title_bar: TitleBarPresenter,
    /// The launcher's name, marked for an assistant, shown after the view's title.
    launcher_title: String,
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
        view: ViewPresenter,
    },
    Disappearing,
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
    /// View-local text input area; `Some` enables the IME while the view is focused.
    pub ime_area: Option<Rect>,
}

impl InstancePresenter {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: InstanceId,
        initial_center_translation: Option<Vector3>,
        show_background: bool,
        root: InstanceRoot,
        parameters: InstanceParameters,
        parent: Handle<Location>,
        kind: InstanceKind,
        launcher_name: &str,
        title_bar_metrics: InstanceTitleBarMetrics,
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
            InstanceKind::Primary => InstanceFullScreen::Primary,
            InstanceKind::Assistant => InstanceFullScreen::Assistant(FullScreenMode::Regular),
        };

        let launcher_title = launcher_title(launcher_name, kind);
        let title_bar = new_title_bar(
            kind,
            title_bar_metrics,
            &launcher_title,
            root.content_location.clone(),
        );

        Self {
            id,
            state: InstancePresenterState::WaitingForPrimaryView,
            parameters,
            movement,
            root,
            target_transform: Transform::from_translation(initial_center_translation),
            has_applied_layout: has_initial_center_translation,
            pacing: RenderPacing::default(),
            background,
            title_bar,
            launcher_title,
            full_screen,
        }
    }

    pub fn parameters(&self) -> &InstanceParameters {
        &self.parameters
    }

    /// The instance's kind (ADR 0014): an assistant carries its own temporary
    /// mode, a primary instance presents in its launcher's mode.
    pub fn kind(&self) -> InstanceKind {
        match self.full_screen {
            InstanceFullScreen::Primary => InstanceKind::Primary,
            InstanceFullScreen::Assistant(_) => InstanceKind::Assistant,
        }
    }

    /// The instance's temporary Full Screen Mode, `None` for a primary instance
    /// (a primary instance presents in its launcher's mode instead).
    pub fn full_screen_mode(&self) -> Option<FullScreenMode> {
        match self.full_screen {
            InstanceFullScreen::Primary => None,
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
            view: ViewPresenter::new(view_creation_info),
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
        let view = self.presented_view_mut(view_id).context(NOT_PRESENTING)?;
        view.window_state.title = title.clone();
        self.title_bar
            .set_text(&title_bar_text(&title, &self.launcher_title));
        Ok(())
    }

    pub fn set_view_cursor(&mut self, view_id: ViewId, cursor: CursorIcon) -> Result<()> {
        let view = self.presented_view_mut(view_id).context(NOT_PRESENTING)?;
        view.window_state.cursor = cursor;
        Ok(())
    }

    pub fn set_view_ime_area(&mut self, view_id: ViewId, area: Option<Rect>) -> Result<()> {
        let view = self.presented_view_mut(view_id).context(NOT_PRESENTING)?;
        view.window_state.ime_area = area;
        Ok(())
    }

    /// This fails if the view is not presented.
    pub fn view_window_state(&self, view_id: ViewId) -> Result<&ViewWindowState> {
        self.presented_view(view_id)
            .map(|view| &view.window_state)
            .context(NOT_PRESENTING)
    }

    pub fn primary_view_id(&self) -> Option<ViewId> {
        Some(self.state.view()?.id())
    }

    /// `None` while the instance presents no view, before it appears or while it disappears. An
    /// instance presents at most one view, so asking for another one is a bug.
    pub fn presented_view_mut(&mut self, view_id: ViewId) -> Option<&mut ViewPresenter> {
        let view = self.state.view_mut()?;
        assert_eq!(view.id(), view_id, "The instance presents another view");
        Some(view)
    }

    pub fn set_view_layout(
        &mut self,
        view_id: ViewId,
        layout: SizedTransform,
    ) -> Result<Option<SizePx>> {
        let view = self.presented_view_mut(view_id).context(NOT_PRESENTING)?;
        let new_size = SizePx::new(layout.size.width as u32, layout.size.height as u32);
        let resize = (view.view_size != new_size).then_some(new_size);
        view.view_size = new_size;

        self.root.view_transform.update_if_changed(layout.transform);

        if let Some(background) = &mut self.background {
            background.update_rect(layout.rect());
        }

        Ok(resize)
    }

    pub fn set_title_bar_layout(&mut self, layout: SizedTransform, animate: bool) {
        self.title_bar.set_layout(layout, animate);
    }

    pub fn set_layout(&mut self, layout: SizedTransform, visible: bool, animate: bool) {
        let snap_layout = !self.has_applied_layout || !animate;

        self.retarget_layout(layout, visible);
        if snap_layout {
            self.movement.snap();
        }
        self.has_applied_layout = true;
    }

    fn retarget_layout(&mut self, layout: SizedTransform, visible: bool) {
        // The content origin is the top-left corner of the extent, while the layout transform is
        // centered on it.
        self.root
            .content_transform
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

    fn presented_view(&self, view_id: ViewId) -> Option<&ViewPresenter> {
        let view = self.state.view()?;
        assert_eq!(view.id(), view_id, "The instance presents another view");
        Some(view)
    }
}

/// The level above the application: instance shortcuts stop here, every other key reaches the
/// application.
impl KeyHandler for InstancePresenter {
    fn handle_key(&self, input: &KeyInput, context: &KeyContext<'_>) -> KeyOutcome {
        match Shortcut::from_input(input) {
            Some(Shortcut::CommandEnter) if !context.fully_zoomed_in => {
                return KeyOutcome::Consumed(DesktopCommand::Zoom(Zoom::Enter).into());
            }
            Some(Shortcut::StartInstance) => {
                return KeyOutcome::Consumed(self.start_instance(input, context).into());
            }
            Some(Shortcut::CloseInstance) => {
                return KeyOutcome::Consumed(self.stop_instance().into());
            }
            _ => {}
        }

        KeyOutcome::Pass
    }
}

/// The instance's extent holds the title bar above the view (ADR 0019); in Full Screen Mode both
/// are laid out at window resolution and scaled into it (ADR 0014).
impl TargetLayout for InstancePresenter {
    type Part = InstancePart;

    fn measure(
        &self,
        part: InstancePart,
        context: &LayoutContext,
        _children: &[MeasuredLayout<2>],
    ) -> MeasuredLayout<2> {
        let size: LayoutSize<2> = match part {
            InstancePart::Instance => {
                instance_extent(context.default_panel_size, context.title_bar_metrics.height).into()
            }
            // The bar spans the view and keeps its regular height on screen, because Full Screen
            // Mode scales it together with the view (ADR 0019).
            InstancePart::TitleBar => SizePx::new(
                self.view_size(context).width,
                context.title_bar_metrics.height,
            )
            .into(),
            InstancePart::View(_) => self.view_size(context).into(),
        };
        size.into()
    }

    fn place_children(
        &self,
        part: InstancePart,
        context: &LayoutContext,
        children: ChildPlacement<'_, Transform, 2>,
    ) {
        let (parent_size, children, placements) = children.into();
        match part {
            InstancePart::Instance => {
                self.place_title_bar_and_view(context, parent_size, children, placements)
            }
            InstancePart::TitleBar | InstancePart::View(_) => {
                assert!(placements.is_empty(), "{part:?} is a leaf")
            }
        }
    }

    fn apply_layout(
        &mut self,
        part: InstancePart,
        layout: SizedTransform,
        visible: bool,
        animate: bool,
    ) -> Result<AppliedLayout> {
        match part {
            InstancePart::Instance => self.set_layout(layout, visible, animate),
            InstancePart::TitleBar => self.set_title_bar_layout(layout, animate),
            InstancePart::View(view) => {
                return Ok(match self.set_view_layout(view, layout)? {
                    Some(size) => AppliedLayout::ViewResized { view, size },
                    None => AppliedLayout::Applied,
                });
            }
        }
        Ok(AppliedLayout::Applied)
    }
}

impl InstancePresenter {
    /// Stacks the title bar and the view vertically. In Full Screen Mode the stack is laid out at
    /// window resolution and scaled into the instance extent (ADR 0014, ADR 0019).
    fn place_title_bar_and_view(
        &self,
        context: &LayoutContext,
        parent_size: LayoutSize<2>,
        children: &[MeasuredLayout<2>],
        placements: &mut [Placement<Transform, 2>],
    ) {
        let (scale, stack_size) = match self.presented_full_screen_mode(context) {
            FullScreenMode::FullScreen => {
                let parent_size_px = SizePx::new(parent_size[0], parent_size[1]);
                (
                    fullscreen_scale(parent_size_px, context.window_size),
                    context.window_size.into(),
                )
            }
            FullScreenMode::Regular => (1.0, parent_size),
        };

        // The scaled stack stays centered in the instance.
        let center =
            |size: LayoutSize<2>| Vector3::new(size[0] as f64 * 0.5, size[1] as f64 * 0.5, 0.0);
        let shift = center(parent_size) - center(stack_size) * scale;

        place_container_children(
            LayoutAxis::VERTICAL,
            0,
            Offset::default(),
            children.iter().map(|child| child.size),
            placements,
        );
        for placement in placements {
            placement.transform = Transform::new(
                placement.transform.translate * scale + shift,
                Quaternion::IDENTITY,
                scale,
            );
        }
    }

    /// The size the view presents at (ADR 0014, ADR 0019).
    fn view_size(&self, context: &LayoutContext) -> SizePx {
        view_size(
            self.presented_full_screen_mode(context),
            context.default_panel_size,
            context.window_size,
            context.title_bar_metrics.height,
        )
    }

    /// An assistant presents in its own temporary mode, a primary instance in its launcher's.
    fn presented_full_screen_mode(&self, context: &LayoutContext) -> FullScreenMode {
        self.full_screen_mode().unwrap_or_else(|| {
            let launcher = context.hierarchy.launcher_of_instance(self.id);
            context.configuration[launcher].full_screen_mode
        })
    }
}

impl InstancePresenter {
    /// Starts an instance under the launcher of this one. `Shift` makes it an assistant: it
    /// spawns without the launcher's configured parameters and carries its own temporary Full
    /// Screen Mode (ADR 0014). A primary instance inherits the parameters of this one.
    fn start_instance(&self, input: &KeyInput, context: &KeyContext<'_>) -> DesktopCommand {
        let kind = InstanceKind::from_modifiers(input.modifiers);
        let parameters = match kind {
            InstanceKind::Primary => self.parameters().clone(),
            InstanceKind::Assistant => InstanceParameters::default(),
        };

        DesktopCommand::StartInstance {
            launcher: context.hierarchy.launcher_of_instance(self.id),
            instance: Uuid::new_v4().into(),
            root: None,
            parameters,
            kind,
        }
    }

    fn stop_instance(&self) -> DesktopCommand {
        // Architecture: Shouldn't this just end the current view, and let the
        // instance decide then?
        DesktopCommand::StopInstance(self.id)
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
    fn view(&self) -> Option<&ViewPresenter> {
        match self {
            Self::WaitingForPrimaryView => None,
            Self::Presenting { view } => Some(view),
            Self::Disappearing => None,
        }
    }

    fn view_mut(&mut self) -> Option<&mut ViewPresenter> {
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

// Title bar height, font size, and horizontal text indent at a scale factor of 1.0.
const TITLE_BAR_HEIGHT_AT_1X: f64 = 24.0;
const TITLE_BAR_FONT_SIZE_AT_1X: f64 = 16.0;
const TITLE_BAR_INDENT_AT_1X: f64 = 8.0;

const PRIMARY_INSTANCE_BACKGROUND_COLOR: Color = Color::rgb_u32(0x1f4f8f);
const ASSISTANT_INSTANCE_BACKGROUND_COLOR: Color = Color::rgb_u32(0x6a3d8f);

/// Creates the bar drawn above an instance's view (ADR 0019): the primary view's title followed by
/// the launcher's title. Assistant instances are set apart by their background color.
fn new_title_bar(
    kind: InstanceKind,
    metrics: InstanceTitleBarMetrics,
    launcher_title: &str,
    parent_location: Handle<Location>,
) -> TitleBarPresenter {
    let background_color = match kind {
        InstanceKind::Primary => PRIMARY_INSTANCE_BACKGROUND_COLOR,
        InstanceKind::Assistant => ASSISTANT_INSTANCE_BACKGROUND_COLOR,
    };
    let style = TitleBarStyle {
        background_color,
        font_size: metrics.font_size,
        indent: metrics.indent,
    };
    let mut title_bar = TitleBarPresenter::new(style, parent_location);
    title_bar.set_text(&title_bar_text("", launcher_title));
    title_bar
}

/// The launcher's name, marked for an assistant instance, which does not run the launcher's
/// configured parameters (ADR 0019).
fn launcher_title(launcher_name: &str, kind: InstanceKind) -> String {
    match kind {
        InstanceKind::Primary => launcher_name.to_string(),
        InstanceKind::Assistant => format!("{launcher_name} (assistant)"),
    }
}

/// The extent of an instance for a regular `panel_size`: its title bar above the panel (ADR 0019).
pub(crate) fn instance_extent(panel_size: SizePx, title_bar_height: u32) -> SizePx {
    SizePx::new(panel_size.width, panel_size.height + title_bar_height)
}

/// The bar's sizes in pixels at regular presentation scale (ADR 0019). They follow the monitor's
/// scale factor, like the terminal's font.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InstanceTitleBarMetrics {
    pub height: u32,
    font_size: f32,
    indent: f64,
}

impl InstanceTitleBarMetrics {
    pub fn from_scale_factor(scale_factor: f64) -> Self {
        Self {
            height: (TITLE_BAR_HEIGHT_AT_1X * scale_factor).round() as u32,
            font_size: (TITLE_BAR_FONT_SIZE_AT_1X * scale_factor) as f32,
            indent: (TITLE_BAR_INDENT_AT_1X * scale_factor).round(),
        }
    }
}

/// Separates the parts of a title, in the title bar as well as in the window title.
pub const TITLE_SEPARATOR: &str = "  ·  ";

/// The title bar's text: the view's title followed by the launcher's title.
fn title_bar_text(view_title: &str, launcher_title: &str) -> String {
    if view_title.is_empty() {
        launcher_title.to_string()
    } else {
        format!("{view_title}{TITLE_SEPARATOR}{launcher_title}")
    }
}
