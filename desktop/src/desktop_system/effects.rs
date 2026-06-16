use std::collections::VecDeque;
use std::ops;
use std::vec;
use strum::{EnumCount, EnumIter, IntoEnumIterator};

use super::DesktopTarget;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DesktopEffect {
    Measure(DesktopTarget),
    Place(DesktopTarget),
    ApplyLayout(DesktopTarget),
    UpdateCamera,
    SyncHover,
    SyncFocusedViewWindowState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumCount, EnumIter)]
#[repr(usize)]
pub enum DesktopEffectPhase {
    Layout,
    PostLayout,
    Finalize,
}

impl DesktopEffect {
    pub const fn phase(&self) -> DesktopEffectPhase {
        match self {
            Self::Measure(_) | Self::Place(_) | Self::ApplyLayout(_) => DesktopEffectPhase::Layout,
            Self::UpdateCamera | Self::SyncHover => DesktopEffectPhase::PostLayout,
            Self::SyncFocusedViewWindowState => DesktopEffectPhase::Finalize,
        }
    }
}

#[must_use]
#[derive(Debug, PartialEq)]
pub struct Effects(Vec<DesktopEffect>);

impl Effects {
    #[allow(non_upper_case_globals)]
    pub const None: Self = Self(Vec::new());
}

impl From<DesktopEffect> for Effects {
    fn from(value: DesktopEffect) -> Self {
        Self(vec![value])
    }
}

impl<const LEN: usize> From<[DesktopEffect; LEN]> for Effects {
    fn from(value: [DesktopEffect; LEN]) -> Self {
        let effects: Vec<DesktopEffect> = value.into();
        Self(effects)
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

#[derive(Debug, Default)]
pub(super) struct DesktopEffectQueue {
    pending_by_phase: [VecDeque<DesktopEffect>; DesktopEffectPhase::COUNT],
}

impl DesktopEffectQueue {
    pub(super) fn enqueue_all(&mut self, effects: Effects) {
        for effect in effects {
            self.enqueue(effect);
        }
    }

    pub(super) fn pop_front(&mut self) -> Option<DesktopEffect> {
        for phase in DesktopEffectPhase::iter() {
            let queue = &mut self.pending_by_phase[phase as usize];
            if let Some(effect) = queue.pop_front() {
                return Some(effect);
            }
        }

        None
    }

    fn enqueue(&mut self, effect: DesktopEffect) {
        let phase = effect.phase();
        let queue = &mut self.pending_by_phase[phase as usize];

        if let Some(index) = queue.iter().position(|pending| pending == &effect) {
            queue.remove(index);
        }

        queue.push_back(effect);
    }
}
