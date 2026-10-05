use massive_util::CollectingSet;

use crate::DesktopTarget;

pub type TargetSet = CollectingSet<DesktopTarget>;

#[derive(Debug, Default)]
pub struct ChangeSurface {
    /// A remeasure is required.
    pub size_invalid: TargetSet,
    pub window_size_changed: bool,
    /// A configuration change was applied to the aggregate; the transaction's
    /// caller persists it (ADR 0013).
    pub configuration_changed: bool,
}

impl ChangeSurface {
    pub fn camera_invalid(&self) -> bool {
        !self.size_invalid.is_empty() || self.window_size_changed
    }

    pub fn retain(&mut self, predicate: impl Fn(&DesktopTarget) -> bool) {
        self.size_invalid.retain(&predicate);
    }

    pub fn combine(&mut self, other: Self) {
        self.size_invalid += other.size_invalid;
        self.window_size_changed |= other.window_size_changed;
        self.configuration_changed |= other.configuration_changed;
    }
}
