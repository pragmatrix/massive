use std::collections::VecDeque;
use std::ops::{self};
use std::vec;

use super::DesktopTarget;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DesktopEffect {
    UpdateLauncherExpansion,
    Measure(DesktopTarget),
    Place(DesktopTarget),
    ApplyLayout(DesktopTarget),
    UpdateCamera,
    SyncHover,
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
    pending: VecDeque<DesktopEffect>,
}

impl DesktopEffectQueue {
    pub(super) fn enqueue_all(&mut self, effects: Effects) {
        for effect in effects {
            self.enqueue(effect);
        }
    }

    pub(super) fn pop_front(&mut self) -> Option<DesktopEffect> {
        self.pending.pop_front()
    }

    fn enqueue(&mut self, effect: DesktopEffect) {
        if let Some(index) = self.pending.iter().position(|pending| pending == &effect) {
            self.pending.remove(index);
        }

        self.pending.push_back(effect);
    }
}
