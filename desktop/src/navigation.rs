#![allow(unused)]
use std::cmp::Ordering;

use massive_geometry::{Point, Rect};

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

pub fn ordered_rects_in_direction<K>(
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
