use std::cmp::Ordering;

use massive_geometry::{PixelCamera, Point, Rect};
use massive_scene::{ToCamera, Transform};

use super::{DesktopSystem, DesktopTarget};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    // Given a 45 degree code starting from center in the direction, return true if the other point
    // is visible. Also returns false if it's the same point.
    pub fn is_visible(&self, center: Point, other: Point) -> bool {
        let dx = other.x - center.x;
        let dy = other.y - center.y;

        match self {
            Direction::Left => dx < 0.0 && dx.abs() >= dy.abs(),
            Direction::Right => dx > 0.0 && dx.abs() >= dy.abs(),
            Direction::Up => dy < 0.0 && dy.abs() >= dx.abs(),
            Direction::Down => dy > 0.0 && dy.abs() >= dx.abs(),
        }
    }
}

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
        direction: Direction,
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

pub(super) fn ordered_rects_in_direction<K>(
    center: Point,
    direction: Direction,
    rects: impl Iterator<Item = (K, Rect)>,
) -> Vec<(K, f64)> {
    let mut results: Vec<(K, f64)> = rects
        .filter_map(|(key, rect)| {
            let rect_center = rect.center();
            direction.is_visible(center, rect_center).then(|| {
                let distance = (rect_center - center).length();
                (key, distance)
            })
        })
        .collect();

    results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
    results
}
