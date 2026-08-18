use massive_util::CollectingSet;

use crate::DesktopTarget;

pub type TargetSet = CollectingSet<DesktopTarget>;

#[derive(Debug, Default)]
pub struct ChangeSurface {
    /// A remeasure is required.
    pub size_invalid: TargetSet,
    /// Targets affected by a focus changes. This is either due to a direct keyboard focus change,
    /// or a change of the [`FocusDepth`] of the current keyboard focused target.
    ///
    /// This decides if the presentation needs to be updated.
    pub presentation_affected: TargetSet,
    pub update_camera: bool,
}

impl ChangeSurface {
    pub fn retain(&mut self, predicate: impl Fn(&DesktopTarget) -> bool) {
        self.size_invalid.retain(&predicate);
        self.presentation_affected.retain(predicate);
    }

    pub fn combine(&mut self, other: Self) {
        self.size_invalid += other.size_invalid;
        self.presentation_affected += other.presentation_affected;
        self.update_camera |= other.update_camera;
    }
}
