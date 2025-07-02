use super::Point;
use flo_curves::{bezier::Normalize, Coordinate};

impl Coordinate for Point {
    fn from_components(components: &[f64]) -> Self {
        Self::new(components[0], components[1])
    }

    fn origin() -> Self {
        Self::new(0.0, 0.0)
    }

    fn len() -> usize {
        2
    }

    fn get(&self, index: usize) -> f64 {
        match index {
            0 => self.x,
            1 => self.y,
            _ => panic!("Invalid coordinate index: {index}"),
        }
    }

    fn from_biggest_components(p1: Self, p2: Self) -> Self {
        Self::new(p1.x.max(p2.x), p1.y.max(p2.y))
    }

    fn from_smallest_components(p1: Self, p2: Self) -> Self {
        Self::new(p1.x.min(p2.x), p1.y.min(p2.y))
    }
}

impl Normalize for Point {
    fn to_normal(_point: &Self, tangent: &Self) -> Vec<f64> {
        vec![-tangent.y, tangent.x]
    }
}
