use std::cmp::max;

use anyhow::Result;

use massive_animation::Movement;
use massive_applications::prelude::*;
use massive_applications::task_context;
use massive_geometry::{Point, SizePx, Transform, TransformedRect};
use massive_layout::{ChildPlacement, LayoutAxis, MeasuredLayout, Offset};
use massive_scene::Id;
use massive_scene::prelude::*;

use crate::desktop_system::change::Zoom;
use crate::desktop_system::{
    AppliedPlacement, DesktopCommand, KeyContext, KeyFocus, KeyHandler, KeyInput, KeyOutcome,
    LayoutContext, McpActivation, ProjectPart, SelectionOperation, Shortcut, TargetLayout,
    expand_cross_axis_child_sizes, expand_main_axis_child_size, measure_container,
    place_container_children,
};
use crate::policy::constants::project;
use crate::projects::{ProjectId, SlotPosition};

use super::animated_placement::AnimatedPlacement;
use super::matrix_layout::{measure_matrix, place_matrix_children};
use super::title_bar_presenter::{TitleBarPresenter, TitleBarStyle};

#[derive(Debug)]
pub struct ProjectPresenter {
    id: ProjectId,
    movement: Movement<AnimatedPlacement>,
    /// The project's own scene location. It is kept so that the project can be attached to
    /// another matrix when its slot moves there.
    location: Handle<Location>,
    pub header: TitleBarPresenter,
    pub matrix: ProjectMatrixPresenter,
    pub last_focused_slot: Option<SlotPosition>,
    /// The corner a new group animates in from, in its parent matrix's coordinates. The group's
    /// first placement takes it. It is `None` from then on, and for every other project.
    entering_from: Option<Point>,
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
            entering_from: None,
        }
    }

    /// Shows a changed header text or MCP activation (ADR 0036).
    pub fn set_header(&mut self, text: &str, mcp_activation: McpActivation) {
        self.header.set_text(text);
        self.header
            .set_background_color(mcp_activation.project_header_background());
    }

    /// Attaches the project to the matrix at `location` and converts its shown placement with
    /// `previous_to_current`, so that it stays where it is on screen. See
    /// [`AnimatedPlacement::rebase`].
    pub fn reparent(&mut self, location: Id<Location>, previous_to_current: Transform) {
        self.location.set_parent(location);
        self.movement.modify(move |animated_placement, _context| {
            animated_placement.rebase(previous_to_current);
        });
    }

    /// The placement the project shows now, relative to its parent matrix. `None` before its first
    /// placement is shown.
    pub fn presented_placement(&self) -> Option<TransformedRect> {
        presented_placement(&self.movement)
    }

    /// Fades the project out where it is shown, and drops it once the fade has ended. Only its
    /// header fades, and it moves along while the project still animates. The rest disappears at
    /// once.
    pub fn fade_out(self) {
        self.header.fade_out();
        self.movement.release_when_settled(());
    }

    /// Lets a new group animate into its own slot from `origin`, the corner of its slots' bounds
    /// in the parent matrix's coordinates (ADR 0026).
    ///
    /// The group starts there at the scale of its first placement, so its header only moves. Its
    /// matrix starts where the parent's matrix is, so the slots stay in place.
    pub fn animate_in_from(&mut self, origin: Point) {
        self.entering_from = Some(origin);
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

    fn set_placement(
        &mut self,
        part: ProjectPart,
        placement: TransformedRect,
        _visible: bool,
        animate: bool,
    ) -> Result<AppliedPlacement> {
        match part {
            ProjectPart::Project => {
                if let Some(origin) = self.entering_from.take() {
                    let start = group_start(origin, placement);
                    animate_in_from(&self.movement, start);
                    animate_in_from(&self.matrix.movement, matrix_start(start));
                    // The project snaps below when the placement does not animate, and its matrix,
                    // which may be placed already, has to snap with it.
                    if !animate {
                        self.matrix.movement.snap();
                    }
                }
                set_placement(&self.movement, placement, animate)
            }
            ProjectPart::Header => self.header.set_placement(placement, animate),
            ProjectPart::Matrix => set_placement(&self.matrix.movement, placement, animate),
        }
        Ok(AppliedPlacement::Applied)
    }
}

/// The matrix below the project's header. Its slots' presenters hang under its location.
#[derive(Debug)]
pub struct ProjectMatrixPresenter {
    movement: Movement<AnimatedPlacement>,
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

    /// The placement the matrix shows now, relative to its project. `None` before its first
    /// placement is shown.
    pub fn presented_placement(&self) -> Option<TransformedRect> {
        presented_placement(&self.movement)
    }
}

/// The mounted movement that animates `scene_transform` to each placement set on it.
fn transform_movement(mut scene_transform: Handle<Transform>) -> Movement<AnimatedPlacement> {
    movement(AnimatedPlacement::default(), move |placement, progress| {
        if let Some(placement) = placement.proceed_with(progress) {
            scene_transform.update_if_changed(placement.pivoted_at_origin());
        }
    })
    .mount()
}

/// The placement `movement` shows now. `None` before its first placement is shown.
fn presented_placement(movement: &Movement<AnimatedPlacement>) -> Option<TransformedRect> {
    task_context::with_animation_and_movement(|_, runtime| {
        runtime.inspect(movement, AnimatedPlacement::presented)
    })
}

/// Sets the placement that `movement` animates to, and snaps it when the change does not animate.
fn set_placement(
    movement: &Movement<AnimatedPlacement>,
    placement: TransformedRect,
    animate: bool,
) {
    movement.modify(move |animated_placement, context| {
        animated_placement.set(context, placement);
    });
    if !animate {
        movement.snap();
    }
}

/// Shows `movement` at `from`, so that it animates from there to its placement. See
/// [`AnimatedPlacement::restart_from`].
fn animate_in_from(movement: &Movement<AnimatedPlacement>, from: TransformedRect) {
    movement
        .modify(move |animated_placement, context| animated_placement.restart_from(context, from));
}

/// The start of a new group at `origin` for its first placement `first`. It keeps the placement's
/// scale and size, so the group only moves into its slot.
fn group_start(origin: Point, first: TransformedRect) -> TransformedRect {
    let transform = first.transform;
    let at_origin = Transform::new(origin.with_z(0.0), transform.rotate, transform.scale);
    TransformedRect::new(at_origin.pivoted_at(first.rect().center()), first.size)
}

/// The matrix's start for a project that starts at `project`. It is the inverse of the project's
/// transform, so that the matrix starts where the parent's matrix is.
fn matrix_start(project: TransformedRect) -> TransformedRect {
    let origin = project.pivoted_at_origin().inverse();
    TransformedRect::new(origin.pivoted_at(project.rect().center()), project.size)
}

#[cfg(test)]
mod tests {
    use massive_geometry::{Quaternion, Size, Vector3};

    use super::*;

    /// A new group starts at its corner but already at its final scale. Starting at scale 1 makes
    /// its header shrink while the group moves in.
    #[test]
    fn a_new_group_starts_at_the_scale_of_its_first_placement() {
        let first = TransformedRect::new(
            Transform::new((900.0, 600.0, 0.0), Quaternion::IDENTITY, 0.8),
            Size::new(400.0, 300.0),
        );

        let start = group_start(Point::new(100.0, 50.0), first);

        let corner = start.pivoted_at_origin();
        assert_eq!(corner.translate, Vector3::new(100.0, 50.0, 0.0));
        assert_eq!(corner.scale, 0.8);
        assert_eq!(start.size, first.size);
    }

    /// Content that grouping moves into a new project keeps its on-screen position without a
    /// conversion, because the new matrix starts where the parent's matrix is.
    #[test]
    fn a_started_matrix_starts_at_the_parents_matrix() {
        let project = TransformedRect::new(
            Transform::from_translation((250.0, 180.0, 0.0)),
            Size::new(400.0, 300.0),
        );

        let matrix = matrix_start(project);

        assert_eq!(
            project.pivoted_at_origin() * matrix.pivoted_at_origin(),
            Transform::IDENTITY
        );
    }
}
