use super::Point;

#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub struct Line {
    pub p1: Point,
    pub p2: Point,
}

impl Line {
    pub fn new(p1: Point, p2: Point) -> Self {
        Self { p1, p2 }
    }

    /// Right-rotating angle x to the right positive, y down positive
    pub fn theta(&self) -> f64 {
        (self.p2.y - self.p1.y).atan2(self.p2.x - self.p1.x)
    }

    pub fn delta(&self) -> Point {
        self.p2 - self.p1
    }

    pub fn point_at_t(&self, t: f64) -> Point {
        self.p1 + self.delta() * t
    }

    pub fn center(&self) -> Point {
        self.p2 + self.delta() / 2.0
    }
}

impl From<Line> for (Point, Point) {
    fn from(l: Line) -> Self {
        (l.p1, l.p2)
    }
}

impl From<(Point, Point)> for Line {
    fn from((p1, p2): (Point, Point)) -> Self {
        Self::new(p1, p2)
    }
}
