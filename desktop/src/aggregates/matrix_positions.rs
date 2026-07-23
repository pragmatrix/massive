use anyhow::{Result, bail};
use derive_more::Index;
use massive_applications::MoveDirection;

use crate::Map;
use crate::projects::{LaunchProfileId, MatrixPlacement};

#[derive(Debug, Clone, Copy)]
pub enum RemoveSlotShiftingPolicy {
    ShiftLeft,
}

#[derive(Debug, Default, Index)]
pub struct MatrixPositions {
    positions: Map<LaunchProfileId, MatrixPlacement>,
}

impl MatrixPositions {
    pub fn is_available(
        &self,
        launchers: impl IntoIterator<Item = LaunchProfileId>,
        placement: MatrixPlacement,
    ) -> bool {
        launchers
            .into_iter()
            .all(|launcher| self.positions.get(&launcher) != Some(&placement))
    }

    pub fn place(
        &mut self,
        launchers: impl IntoIterator<Item = LaunchProfileId>,
        launcher: LaunchProfileId,
        placement: MatrixPlacement,
    ) -> Result<()> {
        if !self.is_available(launchers, placement) {
            bail!(
                "Can't place launcher in occupied matrix slot ({}, {})",
                placement.column,
                placement.row
            );
        }

        self.positions.insert(launcher, placement)
    }

    pub fn shifted_launchers(
        &self,
        launchers: impl IntoIterator<Item = LaunchProfileId>,
        launcher: LaunchProfileId,
        direction: MoveDirection,
    ) -> Result<Vec<(LaunchProfileId, MatrixPlacement)>> {
        let launchers = launchers.into_iter().collect::<Vec<_>>();
        let mut shifted_launchers = vec![(launcher, self.positions[&launcher])];

        loop {
            let (_, leading_placement) = *shifted_launchers
                .last()
                .expect("Shifted launchers are never empty");
            let Some(next_placement) = Self::moved_placement(leading_placement, direction) else {
                bail!("Can't shift launcher beyond the matrix boundary");
            };
            let Some(next_launcher) = launchers
                .iter()
                .copied()
                .find(|candidate| self.positions[candidate] == next_placement)
            else {
                break;
            };
            shifted_launchers.push((next_launcher, next_placement));
        }

        Ok(shifted_launchers
            .into_iter()
            .rev()
            .map(|(launcher, placement)| {
                let placement = Self::moved_placement(placement, direction)
                    .expect("Shifted launcher placement was validated");
                (launcher, placement)
            })
            .collect())
    }

    pub fn moved_placement(
        placement: MatrixPlacement,
        direction: MoveDirection,
    ) -> Option<MatrixPlacement> {
        match direction {
            MoveDirection::Left => placement
                .column
                .checked_sub(1)
                .map(|column| MatrixPlacement {
                    column,
                    row: placement.row,
                }),
            MoveDirection::Right => placement
                .column
                .checked_add(1)
                .map(|column| MatrixPlacement {
                    column,
                    row: placement.row,
                }),
            MoveDirection::Up => placement.row.checked_sub(1).map(|row| MatrixPlacement {
                column: placement.column,
                row,
            }),
            MoveDirection::Down => placement.row.checked_add(1).map(|row| MatrixPlacement {
                column: placement.column,
                row,
            }),
        }
    }

    pub fn remove(&mut self, launcher: &LaunchProfileId) -> Result<()> {
        self.positions.remove(launcher)
    }

    pub fn remove_slot(
        &mut self,
        launchers: impl IntoIterator<Item = LaunchProfileId>,
        placement: MatrixPlacement,
        shifting_policy: RemoveSlotShiftingPolicy,
    ) {
        match shifting_policy {
            RemoveSlotShiftingPolicy::ShiftLeft => {
                for launcher in launchers {
                    let position = self
                        .positions
                        .get_mut(&launcher)
                        .expect("Matrix position missing for launcher");
                    if position.row == placement.row && position.column > placement.column {
                        position.column -= 1;
                    }
                }
            }
        }
    }

    pub fn get(&self, launcher: &LaunchProfileId) -> Option<&MatrixPlacement> {
        self.positions.get(launcher)
    }

    pub fn get_mut(&mut self, launcher: &LaunchProfileId) -> Option<&mut MatrixPlacement> {
        self.positions.get_mut(launcher)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifted_launchers_moves_a_contiguous_run_in_reverse_order() {
        let first = LaunchProfileId::new();
        let second = LaunchProfileId::new();
        let third = LaunchProfileId::new();
        let positions = matrix_positions([
            (first, MatrixPlacement { column: 1, row: 0 }),
            (second, MatrixPlacement { column: 2, row: 0 }),
            (third, MatrixPlacement { column: 3, row: 0 }),
        ]);

        let shifted = positions
            .shifted_launchers([first, second, third], first, MoveDirection::Right)
            .unwrap();

        assert_eq!(
            shifted,
            vec![
                (third, MatrixPlacement { column: 4, row: 0 }),
                (second, MatrixPlacement { column: 3, row: 0 }),
                (first, MatrixPlacement { column: 2, row: 0 }),
            ]
        );
    }

    #[test]
    fn shifted_launchers_stops_at_the_first_empty_slot() {
        let first = LaunchProfileId::new();
        let second = LaunchProfileId::new();
        let positions = matrix_positions([
            (first, MatrixPlacement { column: 1, row: 0 }),
            (second, MatrixPlacement { column: 3, row: 0 }),
        ]);

        let shifted = positions
            .shifted_launchers([first, second], first, MoveDirection::Right)
            .unwrap();

        assert_eq!(
            shifted,
            vec![(first, MatrixPlacement { column: 2, row: 0 })]
        );
    }

    #[test]
    fn shifted_launchers_rejects_left_and_up_boundaries() {
        let launcher = LaunchProfileId::new();
        let positions = matrix_positions([(launcher, MatrixPlacement { column: 0, row: 0 })]);

        assert!(
            positions
                .shifted_launchers([launcher], launcher, MoveDirection::Left)
                .is_err()
        );
        assert!(
            positions
                .shifted_launchers([launcher], launcher, MoveDirection::Up)
                .is_err()
        );
    }

    fn matrix_positions(
        entries: impl IntoIterator<Item = (LaunchProfileId, MatrixPlacement)>,
    ) -> MatrixPositions {
        let mut positions = MatrixPositions::default();
        for (launcher, placement) in entries {
            positions.positions.insert(launcher, placement).unwrap();
        }
        positions
    }
}
