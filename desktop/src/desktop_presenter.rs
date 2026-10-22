use std::sync::Arc;

use anyhow::Result;

use massive_animation::{Animated, AnimationAllocator, AnimationProgress, Movement};
use massive_applications::prelude::*;
use massive_geometry::{Rect, SizePx, SizedTransform, Transform};
use massive_layout::{ChildPlacement, LayoutAxis, MeasuredLayout, Offset, Placement};
use massive_scene::prelude::*;
use massive_shapes::{IntoShape, Shape, StrokeRect};

use crate::desktop_system::change::Zoom;
use crate::desktop_system::{
    AppliedLayout, DesktopCommand, KeyContext, KeyHandler, KeyInput, KeyOutcome, LayoutContext,
    SelectionOperation, Shortcut, TargetLayout, expand_cross_axis_child_sizes, measure_container,
    place_container_children,
};
use crate::policy::colors;
use crate::policy::constants::hover_outline;
use crate::policy::when::{self, PressingEscape};

/// Presents desktop-level visuals and scene anchors.
///
/// Responsibilities:
/// - Provides the shared parent location for launcher and instance presenters.
/// - Presents the hover outline visual.
#[derive(Debug)]
pub struct DesktopPresenter {
    pub location: Handle<Location>,
    hover_movement: Movement<HoverMovement>,
}

impl DesktopPresenter {
    pub fn new(location: Handle<Location>) -> Self {
        let (hover_scene_transform, hover_location) = identity_location().submit();
        let hover_visual = create_hover_shapes(None)
            .into_visual()
            .at(&hover_location)
            .submit();
        let hover_movement = movement(HoverMovement::default(), move |movement, context| {
            movement.update_hover_placement_and_visual(
                context,
                &hover_scene_transform,
                &hover_location,
                &hover_visual,
            );
        })
        .mount();

        Self {
            location,
            hover_movement,
        }
    }

    pub fn set_hover_placement(&self, placement: Option<Placement<Transform, 2>>) {
        self.hover_movement.modify(move |movement, context| {
            movement.set_placement(context, placement);
        });
    }
}

/// The desktop is the outermost level of key delivery: it owns the shortcuts that act on the
/// desktop as a whole.
impl KeyHandler for DesktopPresenter {
    fn handle_key(&self, input: &KeyInput, context: &KeyContext<'_>) -> KeyOutcome {
        let command = match Shortcut::from_input(input) {
            // Projects, launchers, and instances enter themselves; the desktop stands for the
            // root project.
            Some(Shortcut::CommandEnter) if context.is_focused => DesktopCommand::Zoom(Zoom::Enter),
            Some(Shortcut::Navigate(direction)) => DesktopCommand::Navigate(direction),
            Some(Shortcut::ExtendSelection(direction)) => {
                DesktopCommand::ExtendSelection(direction)
            }
            Some(shortcut @ (Shortcut::Escape | Shortcut::CommandEscape)) => {
                match when::pressing_escape(shortcut == Shortcut::CommandEscape, context.focus) {
                    PressingEscape::CollapseSelection => DesktopCommand::CollapseSelection,
                    PressingEscape::Escape => DesktopCommand::Escape,
                    PressingEscape::PassToApplication => return KeyOutcome::Pass,
                }
            }
            Some(Shortcut::ZoomIn) => DesktopCommand::Zoom(Zoom::In),
            Some(Shortcut::ZoomOut) => DesktopCommand::Zoom(Zoom::Out),
            Some(Shortcut::NewLauncher) => {
                DesktopCommand::ConfigureSelection(SelectionOperation::AddLauncher)
            }
            Some(Shortcut::Group) => DesktopCommand::ConfigureSelection(SelectionOperation::Group),
            Some(Shortcut::Ungroup) => {
                DesktopCommand::ConfigureSelection(SelectionOperation::Ungroup)
            }
            _ => return KeyOutcome::Pass,
        };

        KeyOutcome::Consumed(command.into())
    }
}

/// The desktop stacks its content vertically; the root project is its only child.
impl TargetLayout for DesktopPresenter {
    type Part = ();

    fn measure(
        &self,
        (): (),
        _context: &LayoutContext,
        children: &[MeasuredLayout<2>],
    ) -> MeasuredLayout<2> {
        measure_container(
            LayoutAxis::VERTICAL,
            Default::default(),
            0,
            children.iter().map(|child| child.size),
        )
        .into()
    }

    fn place_children(
        &self,
        (): (),
        _context: &LayoutContext,
        children: ChildPlacement<'_, Transform, 2>,
    ) {
        let child_sizes = expand_cross_axis_child_sizes(
            LayoutAxis::VERTICAL,
            Default::default(),
            children.parent_size,
            children.measurements,
        );
        place_container_children(
            LayoutAxis::VERTICAL,
            0,
            Offset::default(),
            child_sizes,
            children.placements,
        );
    }

    /// The desktop is the scene's root: its placement only anchors its children's absolute
    /// placements.
    fn apply_layout(
        &mut self,
        (): (),
        _layout: SizedTransform,
        _visible: bool,
        _animate: bool,
    ) -> Result<AppliedLayout> {
        Ok(AppliedLayout::Applied)
    }
}

#[derive(Debug, Default)]
struct HoverMovement {
    alpha: Animated<f32>,
    layout: Animated<SizedTransform>,
}

impl HoverMovement {
    fn update_hover_placement_and_visual(
        &mut self,
        progress: AnimationProgress,
        hover_scene_transform: &Handle<Transform>,
        hover_location: &Handle<Location>,
        hover_visual: &Handle<Visual>,
    ) {
        let alpha = *self.alpha.proceed_with(progress);
        let layout = *self.layout.proceed_with(progress);
        let local_rect = layout.rect();
        let rect_alpha = (alpha != 0.0).then_some((local_rect, alpha));
        hover_scene_transform.update_if_changed(layout.pivoted_at_origin());

        let visual = create_hover_shapes(rect_alpha)
            .into_visual()
            .at(hover_location)
            .with_decal_order(5);
        hover_visual.update_if_changed(visual);
    }

    fn set_placement(
        &mut self,
        context: &mut dyn AnimationAllocator,
        placement: Option<Placement<Transform, 2>>,
    ) {
        match placement {
            Some(placement) => {
                let size = placement.rect.size;
                let layout =
                    SizedTransform::new(SizePx::new(size[0], size[1]), placement.transform);
                self.alpha.animate_if_changed_with(
                    context,
                    1.0,
                    hover_outline::ANIMATION_DURATION,
                    hover_outline::INTERPOLATION,
                );
                if *self.alpha.latest() == 0.0 {
                    self.layout.snap(layout);
                } else {
                    self.layout.animate_if_changed_with(
                        context,
                        layout,
                        hover_outline::ANIMATION_DURATION,
                        hover_outline::INTERPOLATION,
                    );
                }
            }
            None => self.alpha.animate_if_changed_with(
                context,
                0.0,
                hover_outline::ANIMATION_DURATION,
                hover_outline::INTERPOLATION,
            ),
        }
    }
}

fn create_hover_shapes(rect_alpha: Option<(Rect, f32)>) -> Arc<[Shape]> {
    rect_alpha
        .map(|(rect, alpha)| StrokeRect {
            rect: rect.with_outset(hover_outline::STROKE),
            stroke: hover_outline::STROKE.into(),
            color: colors::hover_outline::OUTLINE.with_alpha(alpha),
        })
        .map(IntoShape::into_shape)
        .into_iter()
        .collect()
}
