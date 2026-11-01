//! The slot selection (ADR 0025, ADR 0030). A selection is a rectangle of slots in one project's
//! matrix. It is stored as two offsets from the focused slot's position, one towards each of its
//! corners, so it always contains the focused slot and follows the keyboard focus. Operations on
//! the selection act on the assigned slots inside the rectangle.

use euclid::{Vector2D, vec2};

use super::change::{Changes, DesktopChange, set_focus};
use super::topology::DesktopTopology;
use super::{DesktopSystem, Direction, FocusableTarget, KeyboardFocusReason};
use crate::projects::{
    MatrixPoint, MatrixRect, MatrixUnit, ProjectId, RuntimeConfiguration, SharedColumnCounts,
    SlotContent, SlotPosition, matrix_bounding_box,
};

impl DesktopSystem {
    /// The desktop state the selection reads.
    pub fn selection_context(&self) -> SelectionContext<'_> {
        SelectionContext {
            hierarchy: &self.aggregates.hierarchy,
            configuration: &self.aggregates.configuration,
            shared_column_counts: &self.aggregates.shared_column_counts,
        }
    }

    /// The project whose matrix holds the selection: the project of the focused slot. `None` when
    /// the focus sits in no slot.
    pub fn project_holding_selection(&self) -> Option<ProjectId> {
        let focused = self.keyboard_focus.focused()?;
        let (_, project, _) = self.selection_context().focused_slot(focused)?;
        Some(project)
    }
}

/// The current selection: its offsets from the focused slot's position. `top_left` counts the
/// columns to the left and the rows above, `bottom_right` the columns to the right and the rows
/// below. The zero offsets select only the focused slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    top_left: Vector2D<u32, MatrixUnit>,
    bottom_right: Vector2D<u32, MatrixUnit>,
}

impl Selection {
    /// The selection whose diagonal runs from the focused slot to the slot at `offset`, with
    /// negative columns and rows towards the left and the top.
    pub fn from_diagonal(offset: Vector2D<i32, MatrixUnit>) -> Self {
        Self {
            top_left: vec2(
                offset.x.min(0).unsigned_abs(),
                offset.y.min(0).unsigned_abs(),
            ),
            bottom_right: vec2(
                offset.x.max(0).unsigned_abs(),
                offset.y.max(0).unsigned_abs(),
            ),
        }
    }

    /// Whether the selection reaches beyond the focused slot.
    pub fn is_extended(&self) -> bool {
        *self != Self::default()
    }

    /// Replaces the selection. The transaction reports this as `ChangeSurface::selection_set`, so
    /// the selection survives the focus and configuration changes planned together with it.
    pub fn set(&mut self, selection: Selection) {
        *self = selection;
    }

    /// Collapses the selection to the focused slot.
    pub fn collapse(&mut self) {
        *self = Self::default();
    }

    /// Plans extending the selection by one slot in `direction` (Shift+Cmd+arrow), starting from
    /// the `focused` target.
    ///
    /// When an instance has the focus, its launcher takes the focus, and the selection starts
    /// there. When the focus sits in no slot, or the step would leave the columns and rows the
    /// matrix lays out (ADR 0035), nothing changes.
    pub fn plan_extend(
        &self,
        direction: Direction,
        focused: Option<FocusableTarget>,
        context: &SelectionContext,
    ) -> Changes {
        let Some(focused) = focused else {
            return Changes::Empty;
        };
        let Some((content, project, slot)) = context.focused_slot(focused) else {
            return Changes::Empty;
        };
        let bounds = context.shared_column_counts.shared_matrix_bounds(
            context
                .configuration
                .project(project)
                .expect("a selection's project is configured"),
        );
        let Some(extended) = self.extended(slot, direction, bounds) else {
            return Changes::Empty;
        };
        plan_selection(extended, focused, content)
    }

    /// Plans selecting the bounding box of all assigned slots of `project` around the `focused`
    /// target (Cmd+A, ADR 0030). The focus stays where it is. Only the project whose matrix holds
    /// the focused slot asks for this, so the focus always sits in a slot of `project`.
    pub fn plan_select_all(
        project: ProjectId,
        focused: Option<FocusableTarget>,
        context: &SelectionContext,
    ) -> Changes {
        let focused = focused.expect("selecting all starts from the keyboard focus");
        let (content, focused_project, slot) = context
            .focused_slot(focused)
            .expect("selecting all starts from a focused slot");
        assert_eq!(
            focused_project, project,
            "selecting all acts on the project of the focused slot"
        );
        let slots = context
            .configuration
            .slots_ordered(project)
            .map(|(slot, _)| slot.into());
        let all = Self::from_rect(slot, matrix_bounding_box(slots));
        plan_selection(all, focused, content)
    }

    /// The project whose assigned slots all lie in the selection around the `focused` target.
    pub fn fully_selected_project(
        &self,
        focused: FocusableTarget,
        context: &SelectionContext,
    ) -> Option<ProjectId> {
        let (project, selected) = self.selected_slots(focused, context)?;
        (selected.len() == context.configuration.slots_ordered(project).count()).then_some(project)
    }

    /// The slots the selection around the `focused` target covers: the project whose matrix
    /// holds them, and its assigned slots inside the selection, in the matrix's slot order. Without
    /// a selection beyond the focused slot, this is the focused slot alone. `None` when the focus
    /// sits in no slot.
    pub fn selected_slots(
        &self,
        focused: FocusableTarget,
        context: &SelectionContext,
    ) -> Option<(ProjectId, Vec<SlotPosition>)> {
        let (project, rect) = self.selected_rect(focused, context)?;
        let slots = context
            .configuration
            .project(project)
            .expect("a selection's project is configured")
            .slot_contents()
            .map(|(slot, _)| slot)
            .filter(|&slot| rect.contains(slot.into()))
            .collect();
        Some((project, slots))
    }

    /// The rectangle the selection around the `focused` target spans, and the project whose matrix
    /// holds it. Without a selection beyond the focused slot, this is the focused slot alone.
    /// `None` when the focus sits in no slot.
    pub fn selected_rect(
        &self,
        focused: FocusableTarget,
        context: &SelectionContext,
    ) -> Option<(ProjectId, MatrixRect)> {
        let (_, project, slot) = context.focused_slot(focused)?;
        Some((project, self.matrix_rect_around(slot)))
    }

    /// The rectangle the selection spans around the `focused` slot's position. The selection stays
    /// inside the matrix, so it never reaches past its top-left slot. Release builds do not check
    /// integer overflow, so the assertion keeps a broken selection from wrapping around silently.
    fn matrix_rect_around(self, focused: SlotPosition) -> MatrixRect {
        let focused = MatrixPoint::from(focused);
        assert!(
            !self.top_left.greater_than(focused.to_vector()).any(),
            "a selection stays inside its matrix"
        );
        matrix_bounding_box([focused - self.top_left, focused + self.bottom_right])
    }

    /// The selection after one step in `direction`, or `None` when the step would leave `bounds`, the
    /// rectangle of the matrix, around the `focused` slot. A step first shrinks the selection from
    /// the opposite side while it reaches beyond the focused slot there, and grows it otherwise
    /// (ADR 0030).
    fn extended(
        self,
        focused: SlotPosition,
        direction: Direction,
        bounds: MatrixRect,
    ) -> Option<Self> {
        let step: Vector2D<i32, MatrixUnit> = match direction {
            Direction::Left => vec2(-1, 0),
            Direction::Right => vec2(1, 0),
            Direction::Up => vec2(0, -1),
            Direction::Down => vec2(0, 1),
        };
        // The step first shrinks the side it points away from, as far as that side reaches, and
        // the rest of it grows the side it points to.
        let backward = (-step).max(Vector2D::zero()).to_u32();
        let forward = step.max(Vector2D::zero()).to_u32();
        let shrunk_top_left = self.top_left.min(forward);
        let shrunk_bottom_right = self.bottom_right.min(backward);
        let extended = Self {
            top_left: self.top_left - shrunk_top_left + (backward - shrunk_bottom_right),
            bottom_right: self.bottom_right - shrunk_bottom_right + (forward - shrunk_top_left),
        };

        Self::from_rect(focused, bounds)
            .contains(extended)
            .then_some(extended)
    }

    /// The selection around the `focused` slot whose rectangle is `rect`, which contains the slot.
    /// It is the inverse of [`Self::matrix_rect_around`].
    fn from_rect(focused: SlotPosition, rect: MatrixRect) -> Self {
        let focused = MatrixPoint::from(focused);
        Self {
            top_left: focused - rect.min,
            bottom_right: rect.max - vec2(1, 1) - focused,
        }
    }

    /// Whether `other` reaches no further than this selection on any side.
    fn contains(self, other: Self) -> bool {
        !other.top_left.greater_than(self.top_left).any()
            && !other.bottom_right.greater_than(self.bottom_right).any()
    }
}

/// Borrows the only state the selection reads: the topology, the configuration, and the shared
/// column counts that bound how far a selection extends (ADR 0035).
#[derive(Debug, Clone, Copy)]
pub struct SelectionContext<'a> {
    pub hierarchy: &'a DesktopTopology,
    pub configuration: &'a RuntimeConfiguration,
    pub shared_column_counts: &'a SharedColumnCounts,
}

impl SelectionContext<'_> {
    /// The slot the `focused` target belongs to: its content, the project whose matrix holds it,
    /// and the slot itself. An instance belongs to its launcher's slot. `None` when the focus sits
    /// in no slot, which is the case for the root project.
    pub fn focused_slot(
        &self,
        focused: FocusableTarget,
    ) -> Option<(SlotContent, ProjectId, SlotPosition)> {
        let content = self.hierarchy.slot_content_of_target(&focused.into())?;
        let (project, slot) = self
            .configuration
            .slot_of_content(content)
            .expect("slot content sits in a slot");
        Some((content, project, slot))
    }
}

/// Plans setting `selection` around the slot of `content`, which the `focused` target belongs to.
/// When an instance has the focus, its launcher takes it, so that the focus and the selection
/// outline mark the same slot (ADR 0025). The selection comes last, so that it survives that
/// focus change.
fn plan_selection(selection: Selection, focused: FocusableTarget, content: SlotContent) -> Changes {
    let mut changes = Changes::Empty;
    if focused != content.focusable_target() {
        changes += set_focus(
            Some(content.focusable_target()),
            KeyboardFocusReason::ExtendSelection,
        );
    }
    changes <<= DesktopChange::SetSelection(selection);
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagonal(columns: i32, rows: i32) -> Selection {
        Selection::from_diagonal(vec2(columns, rows))
    }

    fn selection(top_left: (u32, u32), bottom_right: (u32, u32)) -> Selection {
        Selection {
            top_left: top_left.into(),
            bottom_right: bottom_right.into(),
        }
    }

    /// The selection grows across unassigned slots, but never past the matrix's top-left or
    /// bottom-right corner.
    #[test]
    fn the_selection_extends_one_slot_at_a_time_inside_the_matrix() {
        let focused = SlotPosition { column: 1, row: 0 };
        let bounds = MatrixRect::new(MatrixPoint::origin(), MatrixPoint::new(4, 2));
        let extend = |from: Selection, direction| from.extended(focused, direction, bounds);

        assert_eq!(
            extend(diagonal(0, 0), Direction::Right),
            Some(diagonal(1, 0))
        );
        assert_eq!(
            extend(diagonal(1, 0), Direction::Down),
            Some(diagonal(1, 1))
        );
        assert_eq!(
            extend(diagonal(0, 0), Direction::Left),
            Some(diagonal(-1, 0))
        );
        assert_eq!(extend(diagonal(-1, 0), Direction::Left), None);
        assert_eq!(extend(diagonal(2, 1), Direction::Right), None);
        assert_eq!(extend(diagonal(2, 1), Direction::Down), None);
        assert_eq!(extend(diagonal(0, 0), Direction::Up), None);
    }

    /// A step shrinks the side opposite to it while that side reaches beyond the focused slot, so
    /// a selection around the focused slot shrinks towards it before it grows again (ADR 0030).
    #[test]
    fn a_step_shrinks_the_opposite_side_before_it_grows() {
        let focused = SlotPosition { column: 1, row: 1 };
        let bounds = MatrixRect::new(MatrixPoint::origin(), MatrixPoint::new(3, 3));
        let extend = |from: Selection, direction| from.extended(focused, direction, bounds);
        let all = selection((1, 1), (1, 1));
        let right_half = selection((0, 1), (1, 1));

        assert_eq!(extend(all, Direction::Right), Some(right_half));
        assert_eq!(extend(all, Direction::Up), Some(selection((1, 1), (1, 0))));
        assert_eq!(extend(right_half, Direction::Right), None);
        // The opposite step shrinks the side that is left, as with a one-sided selection.
        assert_eq!(
            extend(right_half, Direction::Left),
            Some(selection((0, 1), (0, 1)))
        );
    }
}
