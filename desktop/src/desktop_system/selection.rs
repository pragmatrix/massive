//! The slot selection (ADR 0025, ADR 0030). A selection is a rectangle of slots in one project's
//! matrix. It is stored as two offsets from the focused slot's position, one towards each of its
//! corners, so it always contains the focused slot and follows the keyboard focus. Operations on
//! the selection act on the assigned slots inside the rectangle.

use std::iter;

use euclid::{Vector2D, vec2};
use massive_geometry::Transform;
use massive_layout::{MeasuredLayout, Placement, Rect as LayoutRect};

use super::change::{Changes, DesktopChange, set_focus};
use super::layout_state::DesktopLayoutState;
use super::topology::DesktopTopology;
use super::{
    DesktopTarget, Direction, FocusableTarget, KeyboardFocusReason, LayoutContext, ProjectPart,
};
use crate::projects::{
    MatrixPoint, MatrixRect, MatrixUnit, ProjectId, RuntimeConfiguration, SlotContent,
    SlotPosition, intervals_origin, matrix_area, matrix_bounding_box,
};

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
    /// there. When the focus sits in no slot, or the step would leave the matrix, nothing changes.
    pub fn plan_extend(
        &self,
        direction: Direction,
        focused: Option<FocusableTarget>,
        hierarchy: &DesktopTopology,
        configuration: &RuntimeConfiguration,
    ) -> Changes {
        let Some(focused) = focused else {
            return Changes::Empty;
        };
        let Some((content, project, slot)) = focused_slot(focused, hierarchy, configuration) else {
            return Changes::Empty;
        };
        let Some(extended) = self.extended(slot, direction, matrix_bounds(project, configuration))
        else {
            return Changes::Empty;
        };

        let mut changes = Changes::Empty;
        if focused != content.focusable_target() {
            changes += set_focus(
                Some(content.focusable_target()),
                KeyboardFocusReason::ExtendSelection,
            );
        }
        changes <<= DesktopChange::SetSelection(extended);
        changes
    }

    /// The project whose assigned slots all lie in the selection around the `focused` target.
    pub fn fully_selected_project(
        &self,
        focused: FocusableTarget,
        hierarchy: &DesktopTopology,
        configuration: &RuntimeConfiguration,
    ) -> Option<ProjectId> {
        let (project, selected) = self.selected_slots(focused, hierarchy, configuration)?;
        (selected.len() == configuration.slots_ordered(project).count()).then_some(project)
    }

    /// The slots the selection around the `focused` target covers: the project whose matrix
    /// holds them, and its assigned slots inside the selection, in the matrix's slot order. Without
    /// a selection beyond the focused slot, this is the focused slot alone. `None` when the focus
    /// sits in no slot.
    pub fn selected_slots(
        &self,
        focused: FocusableTarget,
        hierarchy: &DesktopTopology,
        configuration: &RuntimeConfiguration,
    ) -> Option<(ProjectId, Vec<SlotPosition>)> {
        let (_, project, slot) = focused_slot(focused, hierarchy, configuration)?;
        let rect = self.rect(slot);
        let slots = configuration
            .project(project)
            .expect("a selection's project is configured")
            .slot_contents()
            .map(|(slot, _)| slot)
            .filter(|&slot| rect.contains(slot.into()))
            .collect();
        Some((project, slots))
    }

    /// Where the outline of the selection around the `focused` target goes, in the
    /// coordinates of the layout root. `None` when only the focused slot is selected, so the
    /// outline marks the focused target as usual.
    pub fn outline_placement(
        &self,
        focused: FocusableTarget,
        layout_state: &DesktopLayoutState,
        context: &LayoutContext,
    ) -> Option<Placement<Transform, 2>> {
        if !self.is_extended() {
            return None;
        }
        let (_, project, slot) = focused_slot(focused, context.hierarchy, context.configuration)?;

        let matrix = DesktopTarget::Project(project, ProjectPart::Matrix);
        let children: Vec<MeasuredLayout<2>> = context
            .hierarchy
            .get_nested(&matrix)
            .iter()
            .map(|child| layout_state.measured(child))
            .collect();
        let area = matrix_area(project, context, &children, self.rect(slot))?;

        let matrix_placement = layout_state.absolute_placement(&matrix, context.hierarchy);
        let offset = intervals_origin(
            matrix_placement.rect.size,
            layout_state.measured(&matrix).size,
        ) + area.offset;
        let area_origin = DesktopLayoutState::origin_transform(&matrix_placement)
            * Transform::from_xy(offset[0] as f64, offset[1] as f64);
        Some(Placement::new(
            area_origin.pivoted_at(DesktopLayoutState::layout_local_center(area.size)),
            LayoutRect::new(matrix_placement.rect.offset + offset, area.size),
        ))
    }

    /// The rectangle the selection spans around the `focused` slot's position. The selection stays
    /// inside the matrix, so it never reaches past its top-left slot.
    fn rect(self, focused: SlotPosition) -> MatrixRect {
        let focused = MatrixPoint::from(focused);
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

        Self::filling(focused, bounds)
            .contains(extended)
            .then_some(extended)
    }

    /// The selection around the `focused` slot that fills `bounds`, which contain the slot.
    fn filling(focused: SlotPosition, bounds: MatrixRect) -> Self {
        let focused = MatrixPoint::from(focused);
        Self {
            top_left: focused - bounds.min,
            bottom_right: bounds.max - vec2(1, 1) - focused,
        }
    }

    /// Whether `other` reaches no further than this selection on any side.
    fn contains(self, other: Self) -> bool {
        !other.top_left.greater_than(self.top_left).any()
            && !other.bottom_right.greater_than(self.bottom_right).any()
    }
}

/// The slot the `focused` target belongs to: its content, the project whose matrix holds it, and
/// the slot itself. An instance belongs to its launcher's slot. `None` when the focus sits in
/// no slot, which is the case for the root project.
fn focused_slot(
    focused: FocusableTarget,
    hierarchy: &DesktopTopology,
    configuration: &RuntimeConfiguration,
) -> Option<(SlotContent, ProjectId, SlotPosition)> {
    let content = hierarchy.slot_content_of_target(&focused.into())?;
    let (project, slot) = configuration
        .slot_of_content(content)
        .expect("slot content sits in a slot");
    Some((content, project, slot))
}

/// The rectangle of `project`'s matrix, from its top-left slot to the rightmost column and the
/// lowest row that hold an assigned slot. Its bottom-right slot may be unassigned, because that
/// column and that row may belong to different slots.
fn matrix_bounds(project: ProjectId, configuration: &RuntimeConfiguration) -> MatrixRect {
    let slots = configuration
        .project(project)
        .expect("a selection's project is configured")
        .slot_contents()
        .map(|(position, _)| position.into());
    matrix_bounding_box(iter::once(MatrixPoint::origin()).chain(slots))
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
