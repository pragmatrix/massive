//! Copyright 2012 Google Inc.
//! Copyright 2023 Armin Sander.
//!
//! Ported from m115
//!
//! Use of this source code is governed by a BSD-style license that can be
//! found in the LICENSE file.

#![allow(clippy::needless_range_loop)]

use crate::skia::{Path, Point, Vector};
use path_enums::PathFirstDirection;

use tiny_skia_path::Scalar;

type scalar = f32;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum SegmentType {
    Line,
    Quad,
}

#[derive(Debug, Copy, Clone)]
struct Segment {
    ty: SegmentType,
    pts: [Point; 2],
    pub norms: [Vector; 2],
    pub mid: Vector,
}

impl Segment {
    fn count_points(&self) -> usize {
        match self.ty {
            SegmentType::Line => 1,
            SegmentType::Quad => 2,
        }
    }

    fn end_pt(&self) -> &Point {
        match self.ty {
            SegmentType::Line => &self.pts[0],
            SegmentType::Quad => &self.pts[1],
        }
    }

    fn end_norm(&self) -> &Vector {
        match self.ty {
            SegmentType::Line => &self.norms[0],
            SegmentType::Quad => &self.norms[1],
        }
    }
}

fn center_of_mass(segments: &[Segment], c: &mut Point) -> bool {
    let mut area = 0.0;
    let mut center = Point::zero();
    let count = segments.len();
    let mut p0 = Point::zero();
    if count > 2 {
        // We translate the polygon so that the first point is at the origin.
        // This avoids some precision issues with small area polygons far away
        // from the origin.
        p0 = *segments[0].end_pt();
        let mut pi = Point::zero();
        let mut pj = *segments[1].end_pt() - p0;
        for i in 1..count - 1 {
            pi = pj;
            pj = *segments[i + 1].end_pt() - p0;

            let t = pi.cross(pj);
            area += t;
            center.x += (pi.x + pj.x) * t;
            center.y += (pi.y + pj.y) * t;
        }
    }

    // If the poly has no area then we instead return the average of
    // its points.
    if area.is_nearly_zero() {
        let mut avg = Point::zero();
        for i in 0..count {
            let pt = segments[i].end_pt();
            avg.x += pt.x;
            avg.y += pt.y;
        }
        let denom = 1.0 / count as f32;
        avg.scale(denom);
        *c = avg;
    } else {
        area *= 3.0;
        area = area.invert();
        center.scale(area);
        // undo the translate of p0 to the origin.
        *c = center + p0;
    }
    c.is_finite()
}

fn compute_vectors(
    segments: &mut [Segment],
    fan_pt: &mut Point,
    dir: PathFirstDirection,
    v_count: &mut i32,
    i_count: &mut i32,
) -> bool {
    if !center_of_mass(segments, fan_pt) {
        return false;
    }
    let count = segments.len();

    // Make the normals point towards the outside
    let norm_side = if dir == PathFirstDirection::Ccw {
        point_priv::Side::Right
    } else {
        point_priv::Side::Left
    };

    let mut v_count_64 = 0i64;
    let mut i_count_64 = 0i64;
    // compute normals at all points
    unsafe {
        for a in 0..count {
            let seg_a = &segments[a] as *const Segment;
            let b = (a + 1) % count;
            let seg_b = &mut segments[b] as *mut Segment;

            let mut prev_pt = (*seg_a).end_pt() as *const Point;
            let n = (*seg_b).count_points();
            for p in 0..n {
                (*seg_b).norms[p] = (*seg_b).pts[p] - *prev_pt;
                (*seg_b).norms[p].normalize();
                (*seg_b).norms[p] = point_priv::make_orthog(&(*seg_b).norms[p], norm_side);
                prev_pt = &(*seg_b).pts[p];
            }
            if SegmentType::Line == (*seg_b).ty {
                v_count_64 += 5;
                i_count_64 += 9;
            } else {
                v_count_64 += 6;
                i_count_64 += 12;
            }
        }
    }

    // compute mid-vectors where segments meet. TODO: Detect shallow corners
    // and leave out the wedges and close gaps by stitching segments together.

    unsafe {
        for a in 0..count {
            let seg_a = &segments[a] as *const Segment;
            let b = (a + 1) % count;
            let seg_b = &mut segments[b] as *mut Segment;
            (*seg_b).mid = (*seg_b).norms[0] + *(*seg_a).end_norm();
            (*seg_b).mid.normalize();
            // corner wedges
            v_count_64 += 4;
            i_count_64 += 6;
        }
    }
    if v_count_64 > i32::MAX as i64 || i_count_64 > i32::MAX as i64 {
        return false;
    }
    *v_count = v_count_64 as i32;
    *i_count = i_count_64 as i32;
    true
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum DegenerateTestDataStage {
    Initial,
    Point,
    Line,
    NonDegenerate,
}

#[derive(Copy, Clone)]
struct DegenerateTestData {
    stage: DegenerateTestDataStage,
    first_point: Point,
    line_normal: Vector,
    line_c: scalar,
}

impl DegenerateTestData {
    fn new() -> Self {
        DegenerateTestData {
            stage: DegenerateTestDataStage::Initial,
            first_point: Point::zero(),
            line_normal: Vector::zero(),
            line_c: 0.0,
        }
    }

    fn is_degenerate(&self) -> bool {
        self.stage != DegenerateTestDataStage::NonDegenerate
    }
}

const CLOSE: scalar = 1.0 / 16.0;
const CLOSE_SQD: scalar = CLOSE * CLOSE;

fn update_degenerate_test(data: &mut DegenerateTestData, pt: &Point) {
    match data.stage {
        DegenerateTestDataStage::Initial => {
            data.first_point = *pt;
            data.stage = DegenerateTestDataStage::Point;
        }
        DegenerateTestDataStage::Point => {
            if point_priv::distance_to_sqd(pt, &data.first_point) > CLOSE_SQD {
                data.line_normal = (*pt) - data.first_point;
                data.line_normal.normalize();
                data.line_normal =
                    point_priv::make_orthog(&data.line_normal, point_priv::Side::Left);
                data.line_c = -data.line_normal.dot(data.first_point);
                data.stage = DegenerateTestDataStage::Line;
            }
        }
        DegenerateTestDataStage::Line => {
            if (data.line_normal.dot(*pt) + data.line_c).abs() > CLOSE {
                data.stage = DegenerateTestDataStage::NonDegenerate;
            }
        }
        DegenerateTestDataStage::NonDegenerate => {}
        _ => panic!("Unexpected degenerate test stage."),
    }
}

fn add_line_to_segment(pt: &Point, segments: &mut Vec<Segment>) {
    segments.push(Segment {
        ty: SegmentType::Line,
        pts: [pt.clone(), Point::zero()],
        norms: Default::default(),
        mid: Default::default(),
    });
}

fn add_quad_segment(pts: &[Point], segments: &mut Vec<Segment>) {
    debug_assert!(pts.len() == 3);
    if point_priv::distance_to_line_segment_between_sqd(&pts[1], &pts[0], &pts[2]) < CLOSE_SQD {
        if pts[0] != pts[2] {
            add_line_to_segment(&pts[2], segments);
        }
    } else {
        segments.push(Segment {
            ty: SegmentType::Quad,
            pts: [pts[1], pts[2]],
            norms: Default::default(),
            mid: Default::default(),
        });
    }
}

fn add_cubic_segments(pts: &[Point; 4], dir: PathFirstDirection, segments: &mut Vec<Segment>) {
    let mut quads = vec![Point::zero(); 15];
    path_utils::convert_cubic_to_quads_constrain_to_tangents(pts, 1.0, dir, &mut quads);
    let count = quads.len();
    for q in (0..count).step_by(3) {
        add_quad_segment(&quads[q..q + 3], segments);
    }
}

// From `SkPathEnums.h`

mod path_enums {

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum PathFirstDirection {
        Cw,
        Ccw,
        Unknown,
    }
}

mod point_priv {
    use crate::skia::Point;

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum Side {
        Left = -1,
        On = 0,
        Right = 1,
    }

    pub fn make_orthog(vec: &Point, side: Side) -> Point {
        debug_assert!(side == Side::Right || side == Side::Left);
        if side == Side::Right {
            Point::from_xy(-vec.y, vec.x)
        } else {
            Point::from_xy(vec.y, -vec.x)
        }
    }

    pub fn distance_to_line_segment_between_sqd(pt: &Point, a: &Point, b: &Point) -> f32 {
        // See comments to distanceToLineBetweenSqd. If the projection of c onto
        // u is between a and b then this returns the same result as that
        // function. Otherwise, it returns the distance to the closer of a and
        // b. Let the projection of v onto u be v'.  There are three cases:
        //    1. v' points opposite to u. c is not between a and b and is closer
        //       to a than b.
        //    2. v' points along u and has magnitude less than y. c is between
        //       a and b and the distance to the segment is the same as distance
        //       to the line ab.
        //    3. v' points along u and has greater magnitude than u. c is not
        //       not between a and b and is closer to b than a.
        // v' = (u dot v) * u / |u|. So if (u dot v)/|u| is less than zero we're
        // in case 1. If (u dot v)/|u| is > |u| we are in case 3. Otherwise
        // we're in case 2. We actually compare (u dot v) to 0 and |u|^2 to
        // avoid a sqrt to compute |u|.

        let u = *b - *a;
        let v = *pt - *a;

        let u_length_sqd = length_sqd(&u);
        let u_dot_v = u.dot(v);

        // closest point is point A
        if u_dot_v <= 0.0 {
            return length_sqd(&v);
        // closest point is point B
        } else if u_dot_v > u_length_sqd {
            return distance_to_sqd(&b, &pt);
        // closest point is inside segment
        } else {
            let det = u.cross(v);
            let temp = det / u_length_sqd;
            let temp = temp * det;
            // It's possible we have a degenerate segment, or we're so far away it looks degenerate
            // In this case, return squared distance to point A.
            if !temp.is_finite() {
                return length_sqd(&v);
            }
            return temp;
        }
    }

    fn length_sqd(pt: &Point) -> f32 {
        pt.dot(*pt)
    }

    pub fn distance_to_sqd(pt: &Point, a: &Point) -> f32 {
        let dx = pt.x - a.x;
        let dy = pt.y - a.y;
        dx * dx + dy * dy
    }
}
