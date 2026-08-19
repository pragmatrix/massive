use std::collections::VecDeque;
use std::{ops, vec};

use strum::{EnumCount, EnumIter, IntoEnumIterator};

use super::DesktopTarget;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DesktopEffect {
    Measure(DesktopTarget),
    Place(DesktopTarget),
    ApplyLayout(DesktopTarget),
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, EnumCount, EnumIter,
)]
enum DesktopEffectPhase {
    #[default]
    Layout,
    PropagatePlacements,
}

impl DesktopEffect {
    const fn phase(&self) -> DesktopEffectPhase {
        match self {
            Self::Measure(_) | Self::Place(_) => DesktopEffectPhase::Layout,
            Self::ApplyLayout(_) => DesktopEffectPhase::PropagatePlacements,
        }
    }
}

#[must_use]
#[derive(Debug, Default, PartialEq)]
pub struct Effects(Vec<DesktopEffect>);

impl Effects {
    #[allow(non_upper_case_globals)]
    pub const None: Self = Self(Vec::new());
}

impl<const LEN: usize> From<[DesktopEffect; LEN]> for Effects {
    fn from(value: [DesktopEffect; LEN]) -> Self {
        let effects: Vec<DesktopEffect> = value.into();
        Self(effects)
    }
}

impl FromIterator<DesktopEffect> for Effects {
    fn from_iter<I: IntoIterator<Item = DesktopEffect>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl ops::Add for Effects {
    type Output = Effects;

    fn add(mut self, rhs: Self) -> Self::Output {
        self += rhs;
        self
    }
}

impl ops::AddAssign<DesktopEffect> for Effects {
    fn add_assign(&mut self, rhs: DesktopEffect) {
        self.0.push(rhs);
    }
}

impl ops::AddAssign<Effects> for Effects {
    fn add_assign(&mut self, rhs: Self) {
        self.0.extend(rhs.0);
    }
}

impl IntoIterator for Effects {
    type Item = DesktopEffect;
    type IntoIter = vec::IntoIter<DesktopEffect>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

#[derive(Debug)]
pub struct DesktopEffectScheduler {
    pending_by_phase: [VecDeque<DesktopEffect>; DesktopEffectPhase::COUNT],
    current_phase: DesktopEffectPhase,
}

impl DesktopEffectScheduler {
    pub fn new(initial_effects: Effects) -> Self {
        let mut scheduler = Self {
            pending_by_phase: Default::default(),
            current_phase: DesktopEffectPhase::default(),
        };
        scheduler.enqueue_all(initial_effects);
        scheduler
    }

    pub fn enqueue_all(&mut self, effects: Effects) {
        for effect in effects {
            self.enqueue(effect);
        }
    }

    pub fn pop_next(&mut self) -> Option<DesktopEffect> {
        for phase in DesktopEffectPhase::iter().filter(|phase| *phase >= self.current_phase) {
            let queue = &mut self.pending_by_phase[phase as usize];
            if let Some(effect) = queue.pop_front() {
                self.current_phase = phase;
                return Some(effect);
            }
        }

        None
    }

    /// Schedules an effect after existing work in its phase, moving an equivalent pending effect
    /// to the end; phases already processed cannot receive new effects.
    fn enqueue(&mut self, effect: DesktopEffect) {
        let phase = effect.phase();
        if phase < self.current_phase {
            panic!(
                "Internal error: effect {effect:?} enqueued for completed phase {phase:?} while running {:?}",
                self.current_phase
            );
        }

        let queue = &mut self.pending_by_phase[phase as usize];

        if let Some(index) = queue.iter().position(|pending| pending == &effect) {
            queue.remove(index);
        }

        queue.push_back(effect);
    }
}
