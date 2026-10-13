use massive_applications::InstanceId;

use super::{DesktopFocusPath, DesktopTarget, InstancePart};

impl DesktopFocusPath {
    pub fn instance(&self) -> Option<InstanceId> {
        self.iter().rev().find_map(|t| match t {
            DesktopTarget::Instance(id, InstancePart::Instance) => Some(*id),
            _ => None,
        })
    }

    #[allow(unused)]
    /// Is this or a parent something that can be added new instances to?
    pub fn instance_parent(&self) -> Option<DesktopFocusPath> {
        self.iter()
            .enumerate()
            .rev()
            .find_map(|(i, t)| match t {
                // Pre-fractal: the desktop node cannot parent instances.
                DesktopTarget::Desktop => None,
                DesktopTarget::Project(..) => None,
                DesktopTarget::Launcher(..) => Some(i + 1),
                DesktopTarget::Instance(_, InstancePart::Instance) => Some(i),
                DesktopTarget::Instance(_, InstancePart::TitleBar | InstancePart::View(_)) => {
                    assert!(matches!(
                        self[i - 1],
                        DesktopTarget::Instance(_, InstancePart::Instance)
                    ));
                    Some(i - 1)
                }
            })
            .map(|i| self.iter().take(i).cloned().collect::<Vec<_>>().into())
    }
}
