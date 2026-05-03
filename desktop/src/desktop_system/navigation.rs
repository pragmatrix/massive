use std::cmp::Ordering;

use massive_geometry::{PixelCamera, Point, Rect, RectPx};
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
    fn is_visible(&self, center: Point, other: Point) -> bool {
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
    pub(super) fn camera_for_focus(&self, focus: &DesktopTarget) -> Option<PixelCamera> {
        match focus {
            DesktopTarget::Desktop => {
                let placement = self.placement(&DesktopTarget::Desktop)?;
                let rect: RectPx = placement.rect.into();
                let rect: Rect = rect.into();
                let size = rect.size();
                // The Desktop is the layout root — its transform is T::default() (IDENTITY),
                // not center-based. Compute the center from the rect.
                let center = rect.center();
                let center: Transform = (center.x, center.y, 0.0).into();
                Some(center.to_camera().with_size(size))
            }
            DesktopTarget::Group(_) | DesktopTarget::Launcher(_) => {
                let transform = self.layouter.placement(focus)?.transform;
                let camera_transform: Transform = transform.translate.into();
                Some(camera_transform.to_camera())
            }
            DesktopTarget::Instance(instance_id) => {
                let instance = &self.aggregates.instances[instance_id];
                // Keep camera focus anchored to the intended layout target so new instances
                // are centered immediately even while their visuals animate in.
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

        let from_transform = self.layouter.placement(from)?.transform;
        let from_center = Point::new(from_transform.translate.x, from_transform.translate.y);
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
            .filter_map(|target| {
                let t = self.layouter.placement(&target)?.transform;
                let center = Point::new(t.translate.x, t.translate.y);
                Some((target, center))
            });

        let ordered = ordered_points_in_direction(from_center, direction, navigation_candidates);
        if let Some((nearest, _distance)) = ordered.first() {
            return Some(nearest.clone());
        }
        None
    }
}

pub(super) fn ordered_points_in_direction<K>(
    center: Point,
    direction: Direction,
    candidates: impl Iterator<Item = (K, Point)>,
) -> Vec<(K, f64)> {
    let mut results: Vec<(K, f64)> = candidates
        .filter_map(|(key, candidate_center)| {
            direction.is_visible(center, candidate_center).then(|| {
                let distance = (candidate_center - center).length();
                (key, distance)
            })
        })
        .collect();

    results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
    results
}
