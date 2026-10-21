//! The slot selection (ADR 0025). A selection is a rectangle of placements in one project's
//! matrix. It is stored as a delta from the focused slot's placement, so it always starts at the
//! focused slot and follows the keyboard focus. The selected slots are the occupied placements
//! inside the rectangle.

use super::change::{Changes, DesktopChange, set_focus};
use super::{DesktopSystem, Direction, KeyboardFocusReason};
use crate::projects::{MatrixPlacement, ProjectId, RuntimeConfiguration};

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
}

impl DesktopSystem {
    /// Plans extending the selection one placement in `direction` (Shift+Cmd+arrow).
    ///
    /// When an instance has the focus, its launcher takes the focus, and the selection starts
    /// there. When the focus sits in no slot, or the step would leave the matrix, nothing changes.
    pub fn plan_extend_selection(&self, direction: Direction) -> Changes {
        let Some(focused) = self.keyboard_focus.focused() else {
            return Changes::Empty;
        };
        let Some(content) = self
            .aggregates
            .hierarchy
            .slot_content_of_target(&focused.into())
        else {
            return Changes::Empty;
        };
        let configuration = &self.aggregates.configuration;
        let (project, placement) = configuration
            .slot_of_content(content)
            .expect("slot content sits in a slot");
        let Some(delta) = extended_delta(
            placement,
            self.selection.delta(),
            direction,
            last_placement(project, configuration),
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
}

/// The selection's delta after one step in `direction`, or `None` when the step would leave the
/// matrix. The selection starts at `focused`, and the matrix ends at `last`, the last column and
/// the last row that hold a slot.
fn extended_delta(
    focused: MatrixPlacement,
    delta: SelectionDelta,
    direction: Direction,
    last: MatrixPlacement,
) -> Option<SelectionDelta> {
    let (columns, rows) = match direction {
        Direction::Left => (delta.columns - 1, delta.rows),
        Direction::Right => (delta.columns + 1, delta.rows),
        Direction::Up => (delta.columns, delta.rows - 1),
        Direction::Down => (delta.columns, delta.rows + 1),
    };
    let inside = |start: u32, delta: i32, last: u32| {
        (0..=i64::from(last)).contains(&(i64::from(start) + i64::from(delta)))
    };
    (inside(focused.column, columns, last.column) && inside(focused.row, rows, last.row))
        .then_some(SelectionDelta { columns, rows })
}

/// The last column and the last row that hold a slot in `project`'s matrix. They may belong to
/// different slots.
fn last_placement(project: ProjectId, configuration: &RuntimeConfiguration) -> MatrixPlacement {
    configuration
        .project(project)
        .expect("a selection's project is configured")
        .slot_contents()
        .fold(
            MatrixPlacement { column: 0, row: 0 },
            |last, (placement, _)| MatrixPlacement {
                column: last.column.max(placement.column),
                row: last.row.max(placement.row),
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(columns: i32, rows: i32) -> SelectionDelta {
        SelectionDelta { columns, rows }
    }

    /// The selection grows across empty placements, but never past the first or the last column
    /// and row of the matrix.
    #[test]
    fn the_selection_extends_placement_by_placement_inside_the_matrix() {
        let focused = MatrixPlacement { column: 1, row: 0 };
        let last = MatrixPlacement { column: 3, row: 1 };
        let extend = |from, direction| extended_delta(focused, from, direction, last);

        assert_eq!(extend(delta(0, 0), Direction::Right), Some(delta(1, 0)));
        assert_eq!(extend(delta(1, 0), Direction::Down), Some(delta(1, 1)));
        assert_eq!(extend(delta(0, 0), Direction::Left), Some(delta(-1, 0)));
        assert_eq!(extend(delta(-1, 0), Direction::Left), None);
        assert_eq!(extend(delta(2, 1), Direction::Right), None);
        assert_eq!(extend(delta(2, 1), Direction::Down), None);
        assert_eq!(extend(delta(0, 0), Direction::Up), None);
    }
}
