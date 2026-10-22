use massive_util::CollectingSet;

use crate::DesktopTarget;

pub type TargetSet = CollectingSet<DesktopTarget>;

#[derive(Debug, Default)]
pub struct ChangeSurface {
    /// A remeasure is required.
    pub size_invalid: TargetSet,
    /// The targets that this transaction moved under a different parent. Each of them gets a
    /// `Reparent` effect after layout. That effect converts the target's on-screen position into
    /// the coordinates of its new parent, so that it slides from where it was instead of jumping.
    pub reparented: TargetSet,
    pub window_size_changed: bool,
    /// A configuration change was applied to the aggregate; the transaction emits persistence.
    pub configuration_changed: bool,
    /// The slot selection was set. A selection set by the transaction survives its focus, zoom,
    /// and configuration changes (ADR 0025).
    pub selection_set: bool,
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
        // The camera keeps an extended selection in view, so setting a selection may move it
        // (ADR 0032).
        !self.size_invalid.is_empty() || self.window_size_changed || self.selection_set
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
        self.selection_set |= other.selection_set;
        self.window_fullscreen_changed |= other.window_fullscreen_changed;
    }
}
