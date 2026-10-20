use massive_util::CollectingSet;

use crate::DesktopTarget;

pub type TargetSet = CollectingSet<DesktopTarget>;

#[derive(Debug, Default)]
pub struct ChangeSurface {
    /// A remeasure is required.
    pub size_invalid: TargetSet,
    /// Moved under another parent; each one re-bases its drawn transform once placed there.
    pub reparented: TargetSet,
    pub window_size_changed: bool,
    /// A configuration change was applied to the aggregate; the transaction emits persistence.
    pub configuration_changed: bool,
    pub window_fullscreen_changed: bool,
}

impl ChangeSurface {
    pub fn measure(target: DesktopTarget) -> Self {
        Self {
            size_invalid: target.into(),
            ..Default::default()
        }
    }

    pub fn camera_invalid(&self) -> bool {
        !self.size_invalid.is_empty() || self.window_size_changed
    }

    pub fn retain(&mut self, predicate: impl Fn(&DesktopTarget) -> bool) {
        self.size_invalid.retain(&predicate);
        self.reparented.retain(&predicate);
    }

    pub fn combine(&mut self, other: Self) {
        self.size_invalid += other.size_invalid;
        self.reparented += other.reparented;
        self.window_size_changed |= other.window_size_changed;
        self.configuration_changed |= other.configuration_changed;
        self.window_fullscreen_changed |= other.window_fullscreen_changed;
    }
}
