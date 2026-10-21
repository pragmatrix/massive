//! The slot selection (ADR 0025). A selection is a rectangle of placements in one project's
//! matrix. It is stored as a delta from the focused slot's placement, so it always starts at the
//! focused slot and follows the keyboard focus. The selected slots are the occupied placements
//! inside the rectangle.

use massive_geometry::Transform;
use massive_layout::{MeasuredLayout, Placement, Rect as LayoutRect};

use super::change::{Changes, DesktopChange, set_focus};
use super::layout_state::DesktopLayoutState;
use super::topology::DesktopTopology;
use super::{
    DesktopTarget, Direction, FocusableTarget, KeyboardFocusReason, LayoutContext, ProjectPart,
};
use crate::projects::{
    MatrixPlacement, MatrixRect, ProjectId, RuntimeConfiguration, SlotContent, intervals_origin,
    matrix_area,
};

/// How many columns and rows the selection reaches from the focused slot's placement, negative
/// towards the left and the top. The zero delta selects only the focused slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SelectionDelta {
    pub columns: i32,
    pub rows: i32,
}

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
        self.delta = SelectionDelta::default();
    }

    /// Plans extending the selection one placement in `direction` (Shift+Cmd+arrow), starting from
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
        let Some((content, project, placement)) = focused_slot(focused, hierarchy, configuration)
        else {
            return Changes::Empty;
        };
        let Some(delta) = extended_delta(
            placement,
            self.delta,
            direction,
            matrix_bottom_right(project, configuration),
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

    /// Where the outline of the selection that starts at the `focused` target goes, in the
    /// coordinates of the layout root. `None` when only the focused slot is selected, so the
    /// outline marks the focused target as usual.
    pub fn outline_placement(
        &self,
        focused: FocusableTarget,
        layout_state: &DesktopLayoutState,
        context: &LayoutContext,
    ) -> Option<Placement<Transform, 2>> {
        if self.delta == SelectionDelta::default() {
            return None;
        }
        let (_, project, placement) =
            focused_slot(focused, context.hierarchy, context.configuration)?;

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
            selection_rect(placement, self.delta),
        )?;

        let matrix_placement = layout_state.absolute_placement(&matrix, context.hierarchy);
        let offset = intervals_origin(
            matrix_placement.rect.size,
            layout_state.measured(&matrix).size,
        ) + area.offset;
        let area_origin = DesktopLayoutState::origin_transform(&matrix_placement)
            * Transform::from_xy(offset[0] as f64, offset[1] as f64);
        Some(Placement::new(
            area_origin.to_anchor_space(DesktopLayoutState::layout_local_center(area.size)),
            LayoutRect::new(matrix_placement.rect.offset + offset, area.size),
        ))
    }
}

/// The slot the `focused` target belongs to: its content, the project whose matrix holds it, and
/// its placement there. An instance belongs to its launcher's slot. `None` when the focus sits in
/// no slot, which is the case for the root project.
fn focused_slot(
    focused: FocusableTarget,
    hierarchy: &DesktopTopology,
    configuration: &RuntimeConfiguration,
) -> Option<(SlotContent, ProjectId, MatrixPlacement)> {
    let content = hierarchy.slot_content_of_target(&focused.into())?;
    let (project, placement) = configuration
        .slot_of_content(content)
        .expect("slot content sits in a slot");
    Some((content, project, placement))
}

/// The rectangle that `delta` spans from the `focused` slot's placement.
fn selection_rect(focused: MatrixPlacement, delta: SelectionDelta) -> MatrixRect {
    let column = focused.column.saturating_add_signed(delta.columns);
    let row = focused.row.saturating_add_signed(delta.rows);
    MatrixRect {
        top_left: MatrixPlacement {
            column: focused.column.min(column),
            row: focused.row.min(row),
        },
        bottom_right: MatrixPlacement {
            column: focused.column.max(column),
            row: focused.row.max(row),
        },
    }
}

/// The selection's delta after one step in `direction`, or `None` when the step would leave the
/// matrix. The selection starts at `focused`. The matrix spans from its top-left placement to
/// `bottom_right`.
fn extended_delta(
    focused: MatrixPlacement,
    delta: SelectionDelta,
    direction: Direction,
    bottom_right: MatrixPlacement,
) -> Option<SelectionDelta> {
    let (columns, rows) = match direction {
        Direction::Left => (delta.columns - 1, delta.rows),
        Direction::Right => (delta.columns + 1, delta.rows),
        Direction::Up => (delta.columns, delta.rows - 1),
        Direction::Down => (delta.columns, delta.rows + 1),
    };
    let inside = |start: u32, delta: i32, end: u32| {
        (0..=i64::from(end)).contains(&(i64::from(start) + i64::from(delta)))
    };
    (inside(focused.column, columns, bottom_right.column)
        && inside(focused.row, rows, bottom_right.row))
    .then_some(SelectionDelta { columns, rows })
}

/// The bottom-right corner of `project`'s matrix: the rightmost column and the lowest row that hold
/// a slot. The corner itself may be empty, because the column and the row may belong to different
/// slots.
fn matrix_bottom_right(
    project: ProjectId,
    configuration: &RuntimeConfiguration,
) -> MatrixPlacement {
    configuration
        .project(project)
        .expect("a selection's project is configured")
        .slot_contents()
        .fold(
            MatrixPlacement { column: 0, row: 0 },
            |corner, (placement, _)| MatrixPlacement {
                column: corner.column.max(placement.column),
                row: corner.row.max(placement.row),
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(columns: i32, rows: i32) -> SelectionDelta {
        SelectionDelta { columns, rows }
    }

    /// The selection grows across empty placements, but never past the matrix's top-left or
    /// bottom-right corner.
    #[test]
    fn the_selection_extends_placement_by_placement_inside_the_matrix() {
        let focused = MatrixPlacement { column: 1, row: 0 };
        let bottom_right = MatrixPlacement { column: 3, row: 1 };
        let extend = |from, direction| extended_delta(focused, from, direction, bottom_right);

        assert_eq!(extend(delta(0, 0), Direction::Right), Some(delta(1, 0)));
        assert_eq!(extend(delta(1, 0), Direction::Down), Some(delta(1, 1)));
        assert_eq!(extend(delta(0, 0), Direction::Left), Some(delta(-1, 0)));
        assert_eq!(extend(delta(-1, 0), Direction::Left), None);
        assert_eq!(extend(delta(2, 1), Direction::Right), None);
        assert_eq!(extend(delta(2, 1), Direction::Down), None);
        assert_eq!(extend(delta(0, 0), Direction::Up), None);
    }
}
