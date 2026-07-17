use anyhow::{Result, bail};
use derive_more::Index;

use crate::Map;
use crate::desktop_system::Direction;
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

    pub fn make_slot_available(
        &mut self,
        launchers: impl IntoIterator<Item = LaunchProfileId>,
        placement: MatrixPlacement,
        direction: Direction,
    ) -> Result<()> {
        let launchers: Vec<_> = launchers.into_iter().collect();
        if self.is_available(launchers.iter().copied(), placement) {
            return Ok(());
        }

        let is_shifted = |position: MatrixPlacement| match direction {
            Direction::Left => position.row == placement.row && position.column <= placement.column,
            Direction::Right => {
                position.row == placement.row && position.column >= placement.column
            }
            Direction::Up => position.column == placement.column && position.row <= placement.row,
            Direction::Down => position.column == placement.column && position.row >= placement.row,
        };
        let shifted_position = |position: MatrixPlacement| match direction {
            Direction::Left => position
                .column
                .checked_sub(1)
                .map(|column| MatrixPlacement {
                    column,
                    row: position.row,
                }),
            Direction::Right => position
                .column
                .checked_add(1)
                .map(|column| MatrixPlacement {
                    column,
                    row: position.row,
                }),
            Direction::Up => position.row.checked_sub(1).map(|row| MatrixPlacement {
                column: position.column,
                row,
            }),
            Direction::Down => position.row.checked_add(1).map(|row| MatrixPlacement {
                column: position.column,
                row,
            }),
        };

        let shifted: Vec<_> = launchers
            .into_iter()
            .filter(|launcher| is_shifted(self.positions[launcher]))
            .collect();

        if shifted
            .iter()
            .any(|launcher| shifted_position(self.positions[launcher]).is_none())
        {
            bail!("Can't shift matrix positions {direction:?} beyond the matrix boundary");
        }

        for launcher in shifted {
            let position = self
                .positions
                .get_mut(&launcher)
                .expect("Matrix position missing after collecting launcher");
            *position = shifted_position(*position)
                .expect("Matrix position stopped matching the shifting predicate");
        }

        Ok(())
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

    pub fn move_launcher(
        &mut self,
        launcher: LaunchProfileId,
        placement: MatrixPlacement,
    ) -> Result<()> {
        self.positions.update(launcher, placement)
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
}
