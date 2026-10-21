use std::cmp::max;

use anyhow::Result;

use massive_animation::Movement;
use massive_applications::prelude::*;
use massive_geometry::{Color, SizePx, SizedTransform, Transform};
use massive_layout::{ChildPlacement, LayoutAxis, MeasuredLayout, Offset, Size as LayoutSize};
use massive_scene::prelude::*;

use crate::desktop_system::change::Zoom;
use crate::desktop_system::{
    AppliedLayout, DesktopCommand, KeyContext, KeyHandler, KeyInput, KeyOutcome, LayoutContext,
    ProjectPart, Shortcut, TargetLayout, expand_cross_axis_child_sizes,
    expand_main_axis_child_size, measure_container, place_container_children,
};
use crate::projects::{ProjectId, SlotContent, SlotPosition};

use super::matrix_layout::{measure_matrix, place_matrix_children};
use super::slot_layout::SlotLayout;
use super::title_bar_presenter::{TitleBarPresenter, TitleBarStyle};

const PROJECT_HEADER_FONT_SIZE: f32 = 16.0 * 8.0;
const PROJECT_HEADER_BACKGROUND_COLOR: Color = Color::rgb_u32(0x1f4d3d);
const PROJECT_PADDING: u32 = 10;
const PROJECT_HEADER_MIN_HEIGHT: u32 = 24;
const PROJECT_HEADER_SPACING: u32 = 10;

#[derive(Debug)]
pub struct ProjectPresenter {
    id: ProjectId,
    movement: Movement<SlotLayout>,
    /// The project's own scene location. It is kept so that the project can be attached to
    /// another matrix when its slot moves there.
    location: Handle<Location>,
    pub header: TitleBarPresenter,
    pub matrix: ProjectMatrixPresenter,
    pub last_focused_slot: Option<SlotPosition>,
}

impl ProjectPresenter {
    pub fn new(id: ProjectId, name: String, parent_location: Handle<Location>) -> Self {
        let (scene_transform, location) =
            identity_location().relative_to(&parent_location).submit();
        let mut header = TitleBarPresenter::new(
            TitleBarStyle {
                background_color: PROJECT_HEADER_BACKGROUND_COLOR,
                font_size: PROJECT_HEADER_FONT_SIZE,
                indent: 0.0,
            },
            location.clone(),
        );
        header.set_text(&name);
        let matrix = ProjectMatrixPresenter::new(location.clone());
        let movement = movement(SlotLayout::default(), move |layout, progress| {
            if let Some(layout) = layout.proceed_with(progress) {
                scene_transform.update_if_changed(layout.pivoted_at_origin());
            }
        })
        .mount();

        Self {
            id,
            movement,
            location,
            header,
            matrix,
            last_focused_slot: None,
        }
    }

    pub fn set_parent_location(&self, parent: &Handle<Location>) {
        self.location.update_if_changed_with(|location| {
            location.parent = parent.to_ref().into();
        });
    }

    pub fn set_layout(&mut self, layout: SizedTransform, animate: bool) {
        self.movement.modify(move |slot_layout, context| {
            slot_layout.set(context, layout);
        });
        if !animate {
            self.movement.snap();
        }
    }

    /// Converts the on-screen layout into the current parent's coordinates. See
    /// [`SlotLayout::rebase`].
    pub fn rebase(&self, previous_to_current: Transform) {
        self.movement.modify(move |slot_layout, _context| {
            slot_layout.rebase(previous_to_current);
        });
    }
}

/// A project receives no text input, so Enter, with or without Cmd, enters it while it is the
/// keyboard focus (ADR 0018).
impl KeyHandler for ProjectPresenter {
    fn handle_key(&self, input: &KeyInput, context: &KeyContext<'_>) -> KeyOutcome {
        if !context.is_focused {
            return KeyOutcome::Pass;
        }
        let command = match Shortcut::from_input(input) {
            Some(Shortcut::Enter | Shortcut::CommandEnter) => DesktopCommand::Zoom(Zoom::Enter),
            Some(Shortcut::RemoveSlot) => DesktopCommand::RemoveSlot(SlotContent::Project(self.id)),
            _ => return KeyOutcome::Pass,
        };
        KeyOutcome::Consumed(command.into())
    }
}

/// A project stacks its header above its matrix; the header is at least readable, the matrix holds
/// the slots (ADR 0011).
impl TargetLayout for ProjectPresenter {
    type Part = ProjectPart;

    fn measure(
        &self,
        part: ProjectPart,
        context: &LayoutContext,
        children: &[MeasuredLayout<2>],
    ) -> MeasuredLayout<2> {
        match part {
            ProjectPart::Project => {
                let size = measure_container(
                    LayoutAxis::VERTICAL,
                    (PROJECT_PADDING, PROJECT_PADDING).into(),
                    PROJECT_HEADER_SPACING,
                    children.iter().map(|child| child.size),
                );
                // A project fills its slot (ADR 0023).
                MeasuredLayout::new(size, [true, true])
            }
            // The header stretches across the project's width.
            ProjectPart::Header => {
                let measured = self.header.measured_size();
                let size: LayoutSize<2> = SizePx::new(
                    measured.width,
                    max(measured.height, PROJECT_HEADER_MIN_HEIGHT),
                )
                .into();
                MeasuredLayout::new(size, [true, false])
            }
            // The matrix stretches across the project's width to center its columns in it.
            ProjectPart::Matrix => {
                MeasuredLayout::new(measure_matrix(self.id, context, children), [true, false])
            }
        }
    }

    fn place_children(
        &self,
        part: ProjectPart,
        context: &LayoutContext,
        children: ChildPlacement<'_, Transform, 2>,
    ) {
        match part {
            ProjectPart::Project => {
                let padding = (PROJECT_PADDING, PROJECT_PADDING).into();
                let mut child_sizes: Vec<_> = expand_cross_axis_child_sizes(
                    LayoutAxis::VERTICAL,
                    padding,
                    children.parent_size,
                    children.measurements,
                )
                .collect();
                assert_eq!(
                    child_sizes.len(),
                    2,
                    "a project holds a header and a matrix"
                );
                // The matrix takes the height a stretched project gains, so it can center its
                // intervals in it.
                expand_main_axis_child_size(
                    LayoutAxis::VERTICAL,
                    children.parent_size,
                    children.parent_measured_size,
                    &mut child_sizes,
                    1,
                );
                place_container_children(
                    LayoutAxis::VERTICAL,
                    PROJECT_HEADER_SPACING as i32,
                    Offset::from(padding.leading),
                    child_sizes,
                    children.placements,
                );
            }
            ProjectPart::Header => assert!(children.placements.is_empty(), "{part:?} is a leaf"),
            ProjectPart::Matrix => place_matrix_children(
                self.id,
                context,
                children.parent_size,
                children.parent_measured_size,
                children.measurements,
                children.placements,
            ),
        }
    }

    fn apply_layout(
        &mut self,
        part: ProjectPart,
        layout: SizedTransform,
        _visible: bool,
        animate: bool,
    ) -> Result<AppliedLayout> {
        match part {
            ProjectPart::Project => self.set_layout(layout, animate),
            ProjectPart::Header => self.header.set_layout(layout, animate),
            ProjectPart::Matrix => self.matrix.set_layout(layout),
        }
        Ok(AppliedLayout::Applied)
    }
}

#[derive(Debug)]
pub struct ProjectMatrixPresenter {
    scene_transform: Handle<Transform>,
    location: Handle<Location>,
}

impl ProjectMatrixPresenter {
    pub fn new(parent_location: Handle<Location>) -> Self {
        let (scene_transform, location) =
            identity_location().relative_to(&parent_location).submit();

        Self {
            scene_transform,
            location,
        }
    }

    pub fn location(&self) -> Handle<Location> {
        self.location.clone()
    }

    pub fn set_layout(&mut self, layout: SizedTransform) {
        let scene_transform = layout.pivoted_at_origin();
        self.scene_transform.update_if_changed(scene_transform);
    }
}
