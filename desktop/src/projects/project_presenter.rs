use std::cmp::max;

use anyhow::Result;

use massive_animation::Movement;
use massive_applications::prelude::*;
use massive_applications::task_context;
use massive_geometry::{SizePx, SizedTransform, Transform};
use massive_layout::{ChildPlacement, LayoutAxis, MeasuredLayout, Offset};
use massive_scene::Id;
use massive_scene::prelude::*;

use crate::desktop_system::change::Zoom;
use crate::desktop_system::{
    AppliedLayout, DesktopCommand, KeyContext, KeyFocus, KeyHandler, KeyInput, KeyOutcome,
    LayoutContext, McpActivation, ProjectPart, SelectionOperation, Shortcut, TargetLayout,
    expand_cross_axis_child_sizes, expand_main_axis_child_size, measure_container,
    place_container_children,
};
use crate::policy::constants::project;
use crate::projects::{ProjectId, SlotPosition};

use super::matrix_layout::{measure_matrix, place_matrix_children};
use super::slot_layout::SlotLayout;
use super::title_bar_presenter::{TitleBarPresenter, TitleBarStyle};

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
    pub fn new(
        id: ProjectId,
        header_text: &str,
        mcp_activation: McpActivation,
        parent_location: Id<Location>,
    ) -> Self {
        let (scene_transform, location) = identity_location().relative_to(parent_location).submit();
        let mut header = TitleBarPresenter::new(
            TitleBarStyle {
                background_color: mcp_activation.project_header_background(),
                font_size: project::HEADER_FONT_SIZE,
                indent: 0.0,
            },
            location.id(),
        );
        header.set_text(header_text);
        let matrix = ProjectMatrixPresenter::new(location.id());

        Self {
            id,
            movement: transform_movement(scene_transform),
            location,
            header,
            matrix,
            last_focused_slot: None,
        }
    }

    /// Shows a changed header text or MCP activation (ADR 0036).
    pub fn set_header(&mut self, text: &str, mcp_activation: McpActivation) {
        self.header.set_text(text);
        self.header
            .set_background_color(mcp_activation.project_header_background());
    }

    /// Attaches the project to the matrix at `location` and converts its shown layout with
    /// `previous_to_current`, so that it stays where it is on screen. See [`SlotLayout::rebase`].
    pub fn reparent(&mut self, location: Id<Location>, previous_to_current: Transform) {
        self.location.set_parent(location);
        self.movement.modify(move |slot_layout, _context| {
            slot_layout.rebase(previous_to_current);
        });
    }

    pub fn set_layout(&mut self, layout: SizedTransform, animate: bool) {
        set_layout(&self.movement, layout, animate);
    }

    /// The transform from the project's coordinates into its parent matrix's, as it is presented
    /// now.
    pub fn presented_transform(&self) -> Transform {
        presented_transform(&self.movement)
    }

    /// Shows a new group at `bounds`, the bounds of its slots, so that it animates from there once
    /// it is placed in its own slot (ADR 0026).
    ///
    /// Until then, the matrix is shown where the parent's matrix is, so the slots stay in place.
    pub fn animate_in_from(&self, bounds: SizedTransform) {
        animate_in_from(&self.movement, bounds);
        animate_in_from(&self.matrix.movement, matrix_start(bounds));
        self.header.fade_in();
    }
}

/// A project receives no text input, so Enter, with or without Cmd, enters it while it is the
/// keyboard focus (ADR 0018). Cmd+A selects all slots of the project whose matrix holds the focused
/// slot, unless an instance has the focus, whose application keeps Cmd+A (ADR 0030).
impl KeyHandler for ProjectPresenter {
    fn handle_key(&self, input: &KeyInput, context: &KeyContext<'_>) -> KeyOutcome {
        let selects_all =
            context.focused_slot_project == Some(self.id) && context.focus != KeyFocus::Instance;
        let command = match Shortcut::from_input(input) {
            Some(Shortcut::SelectAll) if selects_all => DesktopCommand::SelectAll(self.id),
            _ if !context.is_focused => return KeyOutcome::Pass,
            Some(Shortcut::Enter | Shortcut::CommandEnter) => DesktopCommand::Zoom(Zoom::Enter),
            Some(Shortcut::RemoveSlot) => {
                DesktopCommand::ConfigureSelection(SelectionOperation::Remove)
            }
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
                    (project::PADDING, project::PADDING).into(),
                    project::HEADER_SPACING,
                    children.iter().map(|child| child.size),
                );
                // A project fills its slot vertically (ADR 0023). Its width already holds the
                // columns it shares with its sibling projects, so it never stretches across
                // (ADR 0035).
                MeasuredLayout::new(size, [false, true])
            }
            // The header stretches across the project's width and elides a name that does not
            // fit. It measures no width, so a project's name never changes its size or, through the
            // shared scale, its siblings' (ADR 0034).
            ProjectPart::Header => {
                let height = max(
                    self.header.measured_size().height,
                    project::HEADER_MIN_HEIGHT,
                );
                MeasuredLayout::new(SizePx::new(0, height).into(), [true, false])
            }
            // The matrix is exactly as wide as its columns, and the project as wide as its matrix,
            // so it never stretches across (ADR 0035). The project gives it the height it gains.
            ProjectPart::Matrix => measure_matrix(self.id, context, children).into(),
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
                let padding = (project::PADDING, project::PADDING).into();
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
                    project::HEADER_SPACING as i32,
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
            ProjectPart::Matrix => set_layout(&self.matrix.movement, layout, animate),
        }
        Ok(AppliedLayout::Applied)
    }
}

/// The matrix below the project's header. Its slots' presenters hang under its location.
#[derive(Debug)]
pub struct ProjectMatrixPresenter {
    movement: Movement<SlotLayout>,
    location: Handle<Location>,
}

impl ProjectMatrixPresenter {
    pub fn new(parent_location: Id<Location>) -> Self {
        let (scene_transform, location) = identity_location().relative_to(parent_location).submit();

        Self {
            movement: transform_movement(scene_transform),
            location,
        }
    }

    pub fn location(&self) -> Id<Location> {
        self.location.id()
    }

    /// The transform from the matrix's coordinates into its project's, as it is presented now.
    pub fn presented_transform(&self) -> Transform {
        presented_transform(&self.movement)
    }
}

/// The mounted movement that animates `scene_transform` to each placement set on it.
fn transform_movement(mut scene_transform: Handle<Transform>) -> Movement<SlotLayout> {
    movement(SlotLayout::default(), move |layout, progress| {
        if let Some(layout) = layout.proceed_with(progress) {
            scene_transform.update_if_changed(layout.pivoted_at_origin());
        }
    })
    .mount()
}

/// The transform the scene location of `movement` presents now, which re-parenting converts with,
/// because a parent that still animates is not at its placement yet. It is the identity before the
/// first layout is presented, like the location itself.
fn presented_transform(movement: &Movement<SlotLayout>) -> Transform {
    task_context::with_animation_and_movement(|_, runtime| {
        runtime.inspect(movement, |layout| {
            layout
                .presented()
                .map_or(Transform::IDENTITY, SizedTransform::pivoted_at_origin)
        })
    })
}

/// Sets the placement that `movement` animates to, and snaps it when the change does not animate.
fn set_layout(movement: &Movement<SlotLayout>, layout: SizedTransform, animate: bool) {
    movement.modify(move |slot_layout, context| {
        slot_layout.set(context, layout);
    });
    if !animate {
        movement.snap();
    }
}

/// Shows `movement` at `from`, so that it animates from there once its target is placed.
fn animate_in_from(movement: &Movement<SlotLayout>, from: SizedTransform) {
    movement.modify(move |slot_layout, _context| {
        *slot_layout = SlotLayout::starting_at(from);
    });
}

/// The matrix's start for a project that starts at `project`. It is the inverse of the project's
/// transform, so that the matrix starts where the parent's matrix is.
fn matrix_start(project: SizedTransform) -> SizedTransform {
    let origin = project.pivoted_at_origin().inverse();
    SizedTransform::new(project.size, origin.pivoted_at(project.rect().center()))
}

#[cfg(test)]
mod tests {
    use massive_geometry::Size;

    use super::*;

    /// Content that grouping moves into a new project keeps its on-screen position without a
    /// conversion, because the new matrix starts where the parent's matrix is.
    #[test]
    fn a_started_matrix_starts_at_the_parents_matrix() {
        let project = SizedTransform::new(
            Size::new(400.0, 300.0),
            Transform::from_translation((250.0, 180.0, 0.0)),
        );

        let matrix = matrix_start(project);

        assert_eq!(
            project.pivoted_at_origin() * matrix.pivoted_at_origin(),
            Transform::IDENTITY
        );
    }
}
