//! The slot selection (ADR 0025). A selection is a rectangle of slots in one project's matrix. It
//! is stored as a delta from the focused slot's position, so it always starts at the focused slot
//! and follows the keyboard focus. Operations on the selection act on the assigned slots inside the
//! rectangle.

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
    SlotPosition, intervals_origin, matrix_area, matrix_rect_covering,
};

/// How many columns and rows the selection reaches from the focused slot's position, negative
/// towards the left and the top. The zero delta selects only the focused slot.
pub type SelectionDelta = Vector2D<i32, MatrixUnit>;

/// The current selection, kept as its delta.
#[derive(Debug, Default)]
pub struct Selection {
    delta: SelectionDelta,
}

impl Selection {
    #[cfg(test)]
    pub fn delta(&self) -> SelectionDelta {
        self.delta
    }

    pub fn set(&mut self, delta: SelectionDelta) {
        self.delta = delta;
    }

    /// Collapses the selection to the focused slot.
    pub fn collapse(&mut self) {
        self.delta = SelectionDelta::zero();
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
        let Some(delta) = extended_delta(
            slot,
            self.delta,
            direction,
            matrix_bounds(project, configuration),
        ) else {
            return Changes::Empty;
        };

        let mut changes = Changes::Empty;
        if focused != content.focusable_target() {
            changes += set_focus(
                Some(content.focusable_target()),
                KeyboardFocusReason::ExtendSelection,
            );
        }
        changes <<= DesktopChange::SetSelection(delta);
        changes
    }

    /// The slots the selection that starts at the `focused` target covers: the project whose matrix
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
        let rect = selection_rect(slot, self.delta);
        let slots = configuration
            .project(project)
            .expect("a selection's project is configured")
            .slot_contents()
            .map(|(slot, _)| slot)
            .filter(|&slot| rect.contains(slot.into()))
            .collect();
        Some((project, slots))
    }

    /// Where the outline of the selection that starts at the `focused` target goes, in the
    /// coordinates of the layout root. `None` when only the focused slot is selected, so the
    /// outline marks the focused target as usual.
    pub fn outline_placement(
        &self,
        focused: FocusableTarget,
        layout_state: &DesktopLayoutState,
        context: &LayoutContext,
    ) -> Option<Placement<Transform, 2>> {
        if self.delta == SelectionDelta::zero() {
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
        let area = matrix_area(
            project,
            context,
            &children,
            selection_rect(slot, self.delta),
        )?;

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

/// The rectangle that `delta` spans from the `focused` slot's position.
fn selection_rect(focused: SlotPosition, delta: SelectionDelta) -> MatrixRect {
    let focused = MatrixPoint::from(focused);
    let reached = MatrixPoint::new(
        focused.x.saturating_add_signed(delta.x),
        focused.y.saturating_add_signed(delta.y),
    );
    matrix_rect_covering([focused, reached])
}

/// The selection's delta after one step in `direction`, or `None` when the slot it reaches
/// from `focused` would leave `bounds`, the rectangle of the matrix.
fn extended_delta(
    focused: SlotPosition,
    delta: SelectionDelta,
    direction: Direction,
    bounds: MatrixRect,
) -> Option<SelectionDelta> {
    let delta = delta
        + match direction {
            Direction::Left => vec2(-1, 0),
            Direction::Right => vec2(1, 0),
            Direction::Up => vec2(0, -1),
            Direction::Down => vec2(0, 1),
        };
    let reached = MatrixPoint::new(
        focused.column.checked_add_signed(delta.x)?,
        focused.row.checked_add_signed(delta.y)?,
    );
    bounds.contains(reached).then_some(delta)
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
    matrix_rect_covering(iter::once(MatrixPoint::origin()).chain(slots))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(columns: i32, rows: i32) -> SelectionDelta {
        vec2(columns, rows)
    }

    /// The selection grows across unassigned slots, but never past the matrix's top-left or
    /// bottom-right corner.
    #[test]
    fn the_selection_extends_one_slot_at_a_time_inside_the_matrix() {
        let focused = SlotPosition { column: 1, row: 0 };
        let bounds = MatrixRect::new(MatrixPoint::origin(), MatrixPoint::new(4, 2));
        let extend = |from, direction| extended_delta(focused, from, direction, bounds);

        assert_eq!(extend(delta(0, 0), Direction::Right), Some(delta(1, 0)));
        assert_eq!(extend(delta(1, 0), Direction::Down), Some(delta(1, 1)));
        assert_eq!(extend(delta(0, 0), Direction::Left), Some(delta(-1, 0)));
        assert_eq!(extend(delta(-1, 0), Direction::Left), None);
        assert_eq!(extend(delta(2, 1), Direction::Right), None);
        assert_eq!(extend(delta(2, 1), Direction::Down), None);
        assert_eq!(extend(delta(0, 0), Direction::Up), None);
    }
}
