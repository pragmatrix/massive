use anyhow::{Result, bail};
use derive_more::Index;

use crate::Map;
use crate::projects::{LaunchProfileId, MatrixPlacement};

#[derive(Debug, Clone, Copy)]
pub enum MakeSlotAvailableShiftingPolicy {
    ShiftRight,
}

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
        shifting_policy: MakeSlotAvailableShiftingPolicy,
    ) -> Result<()> {
        let launchers: Vec<_> = launchers.into_iter().collect();
        if self.is_available(launchers.iter().copied(), placement) {
            return Ok(());
        }

        let shifted: Vec<_> = launchers
            .into_iter()
            .filter(|launcher| {
                self.positions.get(launcher).is_some_and(|position| {
                    position.row == placement.row && position.column >= placement.column
                })
            })
            .collect();

        match shifting_policy {
            MakeSlotAvailableShiftingPolicy::ShiftRight => {
                if shifted
                    .iter()
                    .any(|launcher| self.positions[launcher].column.checked_add(1).is_none())
                {
                    bail!("Can't shift matrix positions right beyond the maximum column");
                }

                for launcher in shifted {
                    self.positions
                        .get_mut(&launcher)
                        .expect("Matrix position missing after collecting launcher")
                        .column += 1;
                }
            }
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
