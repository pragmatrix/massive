use massive_geometry::PixelCamera;
use massive_scene::{ToCamera, Transform};

use super::{DesktopSystem, DesktopTarget};
use crate::navigation::{self, ordered_rects_in_direction};

impl DesktopSystem {
    pub fn camera_for_focus(&self, focus: &DesktopTarget) -> Option<PixelCamera> {
        match focus {
            DesktopTarget::Desktop => self
                .rect(&DesktopTarget::Desktop)
                .map(|rect| rect.to_camera()),
            DesktopTarget::Group(group) => {
                Some(self.aggregates.groups[group].rect.center().to_camera())
            }
            DesktopTarget::Launcher(launcher) => Some(
                self.aggregates.launchers[launcher]
                    .rect
                    .final_value()
                    .center()
                    .to_camera(),
            ),
            DesktopTarget::Instance(instance_id) => {
                let instance = &self.aggregates.instances[instance_id];
                let transform: Transform = instance
                    .layout_transform_animation
                    .final_value()
                    .translate
                    .into();
                Some(transform.to_camera())
            }
            DesktopTarget::View(_) => {
                self.camera_for_focus(self.aggregates.hierarchy.parent(focus)?)
            }
        }
    }

    pub(super) fn locate_navigation_candidate(
        &self,
        from: &DesktopTarget,
        direction: navigation::Direction,
    ) -> Option<DesktopTarget> {
        if !matches!(
            from,
            DesktopTarget::Launcher(..) | DesktopTarget::Instance(..) | DesktopTarget::View(..),
        ) {
            return None;
        }

        let from_rect = self.rect(from)?;
        let launcher_targets_without_instances = self
            .aggregates
            .launchers
            .keys()
            .map(|l| DesktopTarget::Launcher(*l))
            .filter(|t| self.aggregates.hierarchy.get_nested(t).is_empty());
        let all_instances_or_views = self.aggregates.instances.keys().map(|instance| {
            if let Some(view) = self.aggregates.view_of_instance(*instance) {
                DesktopTarget::View(view)
            } else {
                DesktopTarget::Instance(*instance)
            }
        });
        let navigation_candidates = launcher_targets_without_instances
            .chain(all_instances_or_views)
            .filter_map(|target| self.rect(&target).map(|rect| (target, rect)));

        let ordered =
            ordered_rects_in_direction(from_rect.center(), direction, navigation_candidates);
        if let Some((nearest, _rect)) = ordered.first() {
            return Some(nearest.clone());
        }
        None
    }
}
