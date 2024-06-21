use std::ops::{Index, IndexMut};

use serde_tuple::{Deserialize_tuple, Serialize_tuple};

use crate::{Bounds, Line, Point};

#[derive(Clone, Debug)]
pub struct CubicBezier {
    pub start: Point,
    pub span1: Point,
    pub span2: Point,
    pub end: Point,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BezierTail {
    Start,
    End,
}

impl CubicBezier {
    pub fn new(start: Point, span1: Point, span2: Point, end: Point) -> Self {
        Self {
            start,
            span1,
            span2,
            end,
        }
    }

    pub fn from_line_spans(line: Line, span1: Point, span2: Point) -> Self {
        let (start, end) = line.into();
        Self {
            start,
            span1,
            span2,
            end,
        }
    }

    pub fn nearest_t_of_point(&self, p: impl Into<Point>) -> f64 {
        algorithms::nearest_t_on_cubic_bezier(&self.to_points(), p.into())
    }

    pub fn point_at_t(&self, t: f64) -> Point {
        algorithms::at_t(&self.to_points(), t)
    }

    pub fn theta_at_t(&self, t: f64) -> f64 {
        algorithms::theta_at_t(&self.to_points(), t)
    }

    pub fn normal_at_t(&self, t: f64) -> Point {
        algorithms::normal_at_t(&self.to_points(), t)
    }

    pub fn to_points(&self) -> [Point; 4] {
        [self.start, self.span1, self.span2, self.end]
    }

    // Intersection based on point in target geometry test and bisect.
    pub fn intersect_with(&self, at: BezierTail, is_inside: impl Fn(Point) -> bool) -> f64 {
        let (start, fin) = match at {
            BezierTail::Start => (0.0, 1.0),
            BezierTail::End => (1.0, 0.0),
        };

        linear_bisect(
            start,
            fin,
            DEFAULT_INTERSECTION_ITERATIONS,
            |a, b| {
                // TODO: do we really need this, it seems rather expensive?
                let p1 = self.point_at_t(a);
                let p2 = self.point_at_t(b);
                (p2 - p1).length() > DEFAULT_INTERSECTION_TOLERANCE
            },
            |h| {
                let p = self.point_at_t(h);
                is_inside(p)
            },
        )
    }

    /// Marches the given distances relative to `t`. Returns the new `t`.
    ///
    /// - `t` the T we start marching
    /// - `distance` the distance (in point coordinates) to march the bezier's chord point at T
    ///   (positive: direction towards the end of the bezier, negative, towards the start).
    ///
    /// Returns a T that has a distance to the point of currentT (line distance)
    /// TODO: Take a look at flo_curves `walk.fs` module. And if so, consider that the this does not
    /// work on the chords, but instead on the bezier.
    pub fn march(&self, t: f64, distance: f64) -> f64 {
        let p = self.point_at_t(t);
        let abs_distance = distance.abs();

        linear_bisect(
            t,
            if distance >= 0.0 { 1.0 } else { 0.0 },
            DEFAULT_INTERSECTION_ITERATIONS,
            |a, b| {
                let p1 = self.point_at_t(a);
                let p2 = self.point_at_t(b);
                (p2 - p1).length() > DEFAULT_INTERSECTION_TOLERANCE
            },
            |h| {
                let p2 = self.point_at_t(h);
                let dist = (p2 - p).length();
                dist < abs_distance
            },
        )
    }

    /// Move the Point at `t` to `target` on the cubic bezier by adjusting the span points.
    /// TODO: use `Point` arithmetic on `influence`, `influence_factor1`, etc.
    pub fn bend(&self, t: f64, target: impl Into<Point>) -> Self {
        let target = target.into();
        // compute the influence of each of the points.
        let basis = basis_functions::compute(t);

        // TODO: may clamp

        // Decide for a distribution to the spanning points.
        let point_on_bezier = self.point_at_t(t);
        // Effectively we could pass the delta in, or?
        let delta = target - point_on_bezier;

        // Influence factors for now (influence1 + influence2 < 1.0) ... always, because of the two
        // span points never have full influence on the target point.
        let influence1 = basis[1];
        let influence2 = basis[2];

        // The factor of influences what we want distribute on the source points.
        //
        // So, when t == 1/3 the first span point must do everything, when t == 2/3 the second one,
        // this way users get finer grained control over the angles at the ends.
        let mut distribution_factor = (t - (1.0 / 3.0)) * 3.0;

        distribution_factor = distribution_factor.clamp(0.0, 1.0);

        let influence_factor1 = 1.0 - distribution_factor;
        let influence_factor2 = distribution_factor;

        // Now compute the scale of the influence to match 1
        let influence_scale1 = influence_factor1 / influence1;
        let influence_scale2 = influence_factor2 / influence2;

        // This is the delta of the two points.
        let delta1 = delta.scaled(influence_scale1);
        let delta2 = delta.scaled(influence_scale2);

        CubicBezier::new(
            self.start,
            self.span1 + delta1,
            self.span2 + delta2,
            self.end,
        )
    }

    pub fn bounds(&self) -> Bounds {
        algorithms::bounds(&self.to_points())
    }

    /// Bounds by just looking at all the points. Faster, usually a lot larger.
    pub fn control_point_bounds(&self) -> Bounds {
        algorithms::control_point_bounds(&self.to_points())
    }
}

mod basis_functions {
    /// Computes the 4 influence factors of a cubic bezier for a given `t` (influences are always in
    /// the range from `0` to `1.0`) <http://www.gnuplot.info/demo/spline.3.png>
    pub fn compute(t: f64) -> [f64; 4] {
        let omt = 1.0 - t;
        [
            omt * omt * omt,
            3.0 * omt * omt * t,
            3.0 * omt * t * t,
            t * t * t,
        ]
    }
}

const DEFAULT_INTERSECTION_ITERATIONS: usize = 64;
const DEFAULT_INTERSECTION_TOLERANCE: f64 = 0.25;

fn linear_bisect(
    inside: f64,
    outside: f64,
    max_iterations: usize,
    is_significant_change: impl Fn(f64, f64) -> bool,
    is_inside: impl Fn(f64) -> bool,
) -> f64 {
    let mut current = inside;
    let mut next_step = (outside - inside) / 2.0;
    let mut previous = None;

    // Degenerate case: starting value is not actually inside, we exit early then.
    if !is_inside(current) {
        return current;
    }

    for _ in 0..max_iterations {
        if let Some(p) = previous {
            // Terminate early if we haven't moved a significant amount
            if !is_significant_change(p, current) {
                return current;
            }
        }

        previous = Some(current);

        if is_inside(current) {
            current += next_step;
        } else {
            current -= next_step;
        }

        next_step /= 2.0;
    }

    current
}

impl Index<usize> for CubicBezier {
    type Output = Point;

    fn index(&self, index: usize) -> &Self::Output {
        match index {
            0 => &self.start,
            1 => &self.span1,
            2 => &self.span2,
            3 => &self.end,
            _ => panic!("Invalid index {}", index),
        }
    }
}

impl IndexMut<usize> for CubicBezier {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        match index {
            0 => &mut self.start,
            1 => &mut self.span1,
            2 => &mut self.span2,
            3 => &mut self.end,
            _ => panic!("Invalid index {}", index),
        }
    }
}

/// Normalized spans are relative to the positioning at the start / end by projecting them on a
/// coordinate system _on_ a unit line between the starting and ending point of a cubic bezier.
///
/// Normalized spans are useful for storing connectors by making the spans relative to the actual
/// positioning of the start and ending points. This way, the starting and ending points can be
/// moved freely without distorting the shape of the curve.
///
/// TODO: Is "Normalized" the right specifier for that kind of transformed coordinates?
#[derive(Copy, Clone, PartialEq, Default, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct NormalizedSpans {
    pub span1: Point,
    pub span2: Point,
}

impl NormalizedSpans {
    pub fn new(span1: Point, span2: Point) -> Self {
        Self { span1, span2 }
    }

    pub fn denormalize(&self, line: Line) -> (Point, Point) {
        let (param1, param2) = NormalizationParameters::from_line(line);
        let span1 = param1.denormalize(self.span1);
        let span2 = param2.denormalize(self.span2);
        (span1, span2)
    }
}

impl CubicBezier {
    pub fn normalized_spans(&self) -> NormalizedSpans {
        let (param1, param2) =
            NormalizationParameters::from_line(Line::from((self.start, self.end)));
        NormalizedSpans::new(param1.normalize(self.span1), param2.normalize(self.span2))
    }

    pub fn from_line_and_normalized_spans(line: impl Into<Line>, spans: NormalizedSpans) -> Self {
        let line = line.into();
        let (start, end) = line.into();
        let (span1, span2) = spans.denormalize(line);
        Self {
            start,
            span1,
            span2,
            end,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Debug)]
struct NormalizationParameters {
    pub center: Point,
    pub scaling: f64,
    pub rotation: f64,
}

impl NormalizationParameters {
    pub fn new(line: impl Into<Line>, reference_point: f64) -> Self {
        let line = line.into();
        let delta = line.delta();
        let reference = line.p1 + delta * reference_point;
        let scaling = delta.length();
        // The (right) rotation of the line.
        let rotation = line.theta();
        Self {
            center: reference,
            rotation,
            scaling,
        }
    }

    /// Computes the normalization parameters for the two spanning points.
    pub fn from_line(line: impl Into<Line>) -> (Self, Self) {
        // It feels more natural to have the span points initial at start / end (at 1/3, 2/3
        // connectors feel somehow bumpy, unnatural.
        const SPAN1_DEFAULT_T: f64 = 0.0;
        const SPAN2_DEFAULT_T: f64 = 1.0;

        let line = line.into();
        (
            Self::new(line, SPAN1_DEFAULT_T),
            Self::new(line, SPAN2_DEFAULT_T),
        )
    }

    pub fn normalize(&self, p: Point) -> Point {
        (p - self.center)
            .rotated_right(-self.rotation)
            .scaled(1.0 / self.scaling)
    }

    pub fn denormalize(&self, p: Point) -> Point {
        p.scaled(self.scaling).rotated_right(self.rotation) + self.center
    }
}

mod algorithms {
    use super::{Bounds, Line, Point};

    use crate::bezier_algorithms;

    use flo_curves::{
        bezier::Curve, bezier::NormalCurve, BezierCurve, BezierCurveFactory, BoundingBox,
        Coordinate,
    };

    pub fn nearest_t_on_cubic_bezier(points: &[Point], p: Point) -> f64 {
        // Can't use flo_curves for that, because it only returns `t` for points very near the curve.
        // curve(points).t_for_point(&p)
        bezier_algorithms::nearest_t_on_cubic_bezier(points, p)
    }

    pub fn at_t(points: &[Point], t: f64) -> Point {
        curve(points).point_at_pos(t)
    }

    pub fn theta_at_t(points: &[Point], t: f64) -> f64 {
        let p = tangent_at_t(points, t);
        let line = Line::new((0.0, 0.0).into(), p);
        line.theta()
    }

    pub fn tangent_at_t(points: &[Point], t: f64) -> Point {
        curve(points).tangent_at_pos(t)
    }

    /// Returns the normal unit vector at `t`.
    pub fn normal_at_t(points: &[Point], t: f64) -> Point {
        curve(points).normal_at_pos(t).to_unit_vector()
    }

    pub fn bounds(points: &[Point]) -> Bounds {
        let bounds: flo_curves::Bounds<Point> = curve(points).bounding_box();
        Bounds {
            min: bounds.min(),
            max: bounds.max(),
        }
    }

    pub fn control_point_bounds(points: &[Point]) -> Bounds {
        let bounds: flo_curves::Bounds<Point> = curve(points).fast_bounding_box();
        Bounds {
            min: bounds.min(),
            max: bounds.max(),
        }
    }

    fn curve(points: &[Point]) -> Curve<Point> {
        assert!(points.len() == 4);
        Curve::from_points(points[0], (points[1], points[2]), points[3])
    }
}
