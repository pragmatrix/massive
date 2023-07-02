//! Port of bezier algorithms.
//! TODO: Fix clippy suppressions as soon we do have stable baseline tests (manual or automatic)
use crate::Point;

// Solving the Nearest Point-on-Curve Problem and
// A Bezier Curve-Based Root-Finder
// by Philip J. Schneider
// from "Graphics Gems", Academic Press, 1990

const MAXDEPTH: usize = 64; // Maximum depth for recursion

// #define	EPSILON	(ldexp(1.0,-MAXDEPTH-1)) /*Flatness control value */
// 1 * (2 ^ (-65))
#[allow(clippy::excessive_precision)]
const EPSILON: f64 = 2.7105054312137610850186320021749e-20;

const DEGREE: usize = 3;
const W_DEGREE: usize = 5;

pub fn nearest_point_on_cubic_bezier(v: &[Point], p: Point) -> Point {
    let t = nearest_t_on_cubic_bezier(v, p);
    bezier(v, DEGREE, t, None, None)
}

pub fn nearest_t_on_cubic_bezier(b: &[Point], p: Point) -> f64 {
    // Parameter value of closest point
    let mut t;

    // Convert problem to 5th-degree Bezier form
    let w = convert_to_bezier_form(p, b);

    // Find all possible roots of 5th-degree equation
    let mut t_candidate = [0.0; W_DEGREE]; // Possible roots
    let n_solutions = find_roots(&w, W_DEGREE, &mut t_candidate, 0);

    // Compare distances of P to all candidates, and to t=0, and t=1
    {
        let mut new_dist;

        // Check distance to beginning of curve, where t = 0
        let mut v = p - b[0];
        let mut dist = v.squared_length();
        t = 0.0;

        // Find distances for candidate points
        #[allow(clippy::needless_range_loop)]
        for i in 0..n_solutions {
            let bp = bezier(b, DEGREE, t_candidate[i], None, None);
            v = p - bp;
            new_dist = v.squared_length();
            if new_dist < dist {
                dist = new_dist;
                t = t_candidate[i];
            }
        }

        // Finally, look at distance to end point, where t = 1.0
        let v = p - b[DEGREE];
        new_dist = v.squared_length();

        #[allow(unused_assignments)]
        if new_dist < dist {
            dist = new_dist;
            t = 1.0;
        }
    }

    t
}

/// Given a point and a Bezier curve, generate a 5th-degree Bezier-format equation whose solution
/// finds the point on the curve nearest the user-defined point.
///
/// - `p` The point to find t for
/// - `v` The control points
fn convert_to_bezier_form(p: Point, v: &[Point]) -> [Point; W_DEGREE + 1] {
    // Determine the c's -- these are vectors created by subtracting point P from each of the
    // control points
    let mut c = [Point::default(); DEGREE + 1];
    for i in 0..=DEGREE {
        // v(i)'s - p
        c[i] = v[i] - p;
    }
    // Determine the d's -- these are vectors created by subtracting each control point from the
    // next
    let mut d = [Point::default(); DEGREE];
    for i in 0..=DEGREE - 1 {
        // v(i+1) - v(i)
        d[i] = (v[i + 1] - v[i]) * 3.0;
    }

    // Create the c,d table -- this is a table of dot products of the c's and d's
    let mut cd_table = [[0.0; 4]; 3];
    // Dot product of c, d
    for row in 0..=DEGREE - 1 {
        #[allow(clippy::needless_range_loop)]
        for column in 0..=DEGREE {
            cd_table[row][column] = dot(d[row], c[column]);
        }
    }

    // Now, apply the z's to the dot products, on the skew diagonal
    // Also, set up the x-values, making these "points"
    let mut w = [Point::default(); W_DEGREE + 1];
    #[allow(clippy::needless_range_loop)]
    for i in 0..=W_DEGREE {
        w[i] = Point::new(i as f64 / W_DEGREE as f64, 0.0);
    }

    let n = DEGREE;
    let m = DEGREE - 1;
    for k in 0..=n + m {
        let lb = 0.max(k as isize - m as isize) as usize;
        let ub = k.min(n);
        for i in lb..=ub {
            let j = k - i;
            w[i + j].y += cd_table[j][i] * Z[j][i];
        }
    }

    return w;

    // Precomputed "z" for cubics
    static Z: [[f64; 4]; 3] = [
        [1.0, 0.6, 0.3, 0.1],
        [0.4, 0.6, 0.6, 0.4],
        [0.1, 0.3, 0.6, 1.0],
    ];
}

/// Given a 5th-degree equation in Bernstein-Bezier form, find all of the roots in the interval [0,
/// 1].  Return the number of roots found.
///
/// - `w` The control points
/// - `degree` The degree of the polynomial
/// - `t` RETURN candidate t-values
/// - `depth` The depth of the recursion
fn find_roots(w: &[Point], degree: usize, t: &mut [f64], depth: usize) -> usize {
    match crossing_count(w, degree) {
        0 => {
            // No solutions here
            return 0;
        }
        1 => {
            // Unique solution
            // Stop recursion when the tree is deep enough
            // if deep enough, return 1 solution at midpoint
            if depth >= MAXDEPTH {
                t[0] = (w[0].x + w[W_DEGREE].x) / 2.0;
                return 1;
            }
            if control_polygon_flat_enough(w, degree) {
                t[0] = compute_x_intercept(w, degree);
                return 1;
            }
        }
        _ => (),
    }

    // Otherwise, solve recursively after subdividing control polygon

    // New left and right Control polygons
    let mut left = [Point::default(); W_DEGREE + 1];
    let mut right = [Point::default(); W_DEGREE + 1];
    bezier(w, degree, 0.5, Some(&mut left), Some(&mut right));
    let mut left_t = [0.0; W_DEGREE + 1];
    let mut right_t = [0.0; W_DEGREE + 1];
    let left_count = find_roots(&left, degree, &mut left_t, depth + 1);
    let right_count = find_roots(&right, degree, &mut right_t, depth + 1);

    #[allow(clippy::manual_memcpy)]
    // Gather solutions together
    for i in 0..left_count {
        t[i] = left_t[i];
    }
    #[allow(clippy::manual_memcpy)]
    for i in 0..right_count {
        t[i + left_count] = right_t[i];
    }

    // Send back total number of solutions
    left_count + right_count
}

/// Count the number of times a Bezier control polygon crosses the 0-axis. This number is >= the
/// number of roots.
///
/// - `v` Control points of Bezier curve
/// - `degree` Degree of Bezier curve
fn crossing_count(v: &[Point], degree: usize) -> usize {
    // Number of zero-crossings
    let mut n_crossings = 0;
    // Sign of coefficients
    let mut sign = v[0].y.signum();
    let mut old_sign = sign;
    #[allow(clippy::needless_range_loop)]
    for i in 1..=degree {
        sign = v[i].y.signum();
        if sign != old_sign {
            n_crossings += 1;
        };
        old_sign = sign;
    }

    n_crossings
}

/// Check if the control polygon of a Bezier curve is flat enough for recursive subdivision to
/// bottom out.
///
/// - `v` Control points
/// - `degree` Degree of polynomiaL
fn control_polygon_flat_enough(v: &[Point], degree: usize) -> bool {
    // Coefficients of implicit eqn for line from V[0]-V[deg]
    let (a, b, c);

    // Find the  perpendicular distance from each interior control point to line connecting `v[0]` and
    // `v[degree]`
    let mut distance = vec![0.0; degree + 1];
    {
        // Derive the implicit equation for line connecting first and last control points
        a = v[0].y - v[degree].y;
        b = v[degree].x - v[0].x;
        c = v[0].x * v[degree].y - v[degree].x * v[0].y;

        let ab_squared = (a * a) + (b * b);

        for i in 1..degree {
            // Compute distance from each of the points to that line
            distance[i] = a * v[i].x + b * v[i].y + c;
            if distance[i] > 0.0 {
                distance[i] = (distance[i] * distance[i]) / ab_squared;
            }
            if distance[i] < 0.0 {
                distance[i] = -((distance[i] * distance[i]) / ab_squared);
            }
        }
    }

    // Find the largest distance
    let mut max_distance_above: f64 = 0.0;
    let mut max_distance_below: f64 = 0.0;
    #[allow(clippy::needless_range_loop)]
    for i in 1..degree {
        if distance[i] < 0.0 {
            max_distance_below = max_distance_below.min(distance[i]);
        };
        if distance[i] > 0.0 {
            max_distance_above = max_distance_above.max(distance[i]);
        }
    }

    let (intercept_1, intercept_2);
    {
        // Implicit equation for zero line
        let a1 = 0.0;
        let b1 = 1.0;
        let c1 = 0.0;

        // Implicit equation for "above" line
        let mut a2 = a;
        let mut b2 = b;
        let mut c2 = c + max_distance_above;

        let mut det = a1 * b2 - a2 * b1;
        let mut d_inv = 1.0 / det;

        intercept_1 = (b1 * c2 - b2 * c1) * d_inv;

        // Implicit equation for "below" line
        a2 = a;
        b2 = b;
        c2 = c + max_distance_below;

        det = a1 * b2 - a2 * b1;
        d_inv = 1.0 / det;

        intercept_2 = (b1 * c2 - b2 * c1) * d_inv;
    }

    /* Compute intercepts of bounding box	*/
    let left_intercept = intercept_1.min(intercept_2);
    let right_intercept = intercept_1.max(intercept_2);

    // Precision of root
    let error = 0.5 * (right_intercept - left_intercept);
    error < EPSILON
}

/// Compute intersection of chord from first control point to last with 0-axis.
/// NOTE: "T" and "Y" do not have to be computed, and there are many useless
/// operations in the following (e.g. "0.0 - 0.0").
///
/// - `v` Control points
/// - `degree` Degree of curve
fn compute_x_intercept(v: &[Point], degree: usize) -> f64 {
    let xlk = 1.0;
    let ylk = 0.0;
    let xnm = v[degree].x - v[0].x;
    let ynm = v[degree].y - v[0].y;
    let xmk = v[0].x - 0.0;
    let ymk = v[0].y - 0.0;

    let det = xnm * ylk - ynm * xlk;
    let det_inv = 1.0 / det;

    let s = (xnm * ymk - ynm * xmk) * det_inv;
    // T = (XLK*YMK - YLK*XMK) * detInv;

    #[allow(clippy::let_and_return)]
    let x = 0.0 + xlk * s;
    // Y = 0.0 + YLK * S;
    x
}

/// Evaluate a Bezier curve at a particular parameter value Fill in control points for resulting
/// sub-curves if "Left" and "Right" are non-null.
///
/// - `v` Control points
/// - `degree` Degree of bezier curve
/// - `t` Parameter value
/// - `left` RETURN left half ctl pts
/// - `right` RETURN right half ctl pts
fn bezier(
    v: &[Point],
    degree: usize,
    t: f64,
    left: Option<&mut [Point]>,
    right: Option<&mut [Point]>,
) -> Point {
    let mut v_temp = [[Point::default(); W_DEGREE + 1]; W_DEGREE + 1];

    // Copy control points
    #[allow(clippy::manual_memcpy)]
    for j in 0..=degree {
        v_temp[0][j] = v[j];
    }

    // Triangle computation
    for i in 1..=degree {
        for j in 0..=degree - i {
            v_temp[i][j] = Point::new(
                (1.0 - t) * v_temp[i - 1][j].x + t * v_temp[i - 1][j + 1].x,
                (1.0 - t) * v_temp[i - 1][j].y + t * v_temp[i - 1][j + 1].y,
            );
        }
    }

    if let Some(left) = left {
        for j in 0..=degree {
            left[j] = v_temp[j][0];
        }
    }

    if let Some(right) = right {
        #[allow(clippy::manual_memcpy)]
        for j in 0..=degree {
            right[j] = v_temp[degree - j][j];
        }
    }

    v_temp[degree][0]
}

fn dot(a: Point, b: Point) -> f64 {
    a.x * b.x + a.y * b.y
}
