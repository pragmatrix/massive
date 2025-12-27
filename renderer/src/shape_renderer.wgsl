// Vertex shader

struct PushConstants {
    view_model: mat4x4<f32>,
    clip_rect_x: vec2<f32>, // [min_x, max_x]
    clip_rect_y: vec2<f32>, // [min_y, max_y]
}

var<push_constant> pc: PushConstants;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) unorm_tex_coords: vec2<f32>,
    // Per-vertex shape size (width, height)
    @location(3) shape_size: vec2<f32>,
    // Shape selector (0 = rect, 1 = rounded rect, 2 = circle, 3 = rect stroke)
    @location(2) shape_selector: u32,
    // Per-shape vec
    //  - rounded rect: radius (only [0] is used)
    //  - rect stroke: stroke thickness per axis (x = horizontal edges thickness, y = vertical edges thickness)
    @location(4) shape_data: vec2<f32>,
    @location(5) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) unorm_tex_coords: vec2<f32>,
    @location(2) @interpolate(flat) shape_size: vec2<f32>,
    @location(1) @interpolate(flat) shape_selector: u32,
    @location(3) @interpolate(flat) shape_data: vec2<f32>,
    @location(4) @interpolate(flat) color: vec4<f32>,
    @location(5) model_pos: vec2<f32>,
}

@vertex
fn vs_main(
    vertex_input: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.unorm_tex_coords = vertex_input.unorm_tex_coords;
    out.model_pos = vertex_input.position.xy;
    out.clip_position = pc.view_model * vec4<f32>(vertex_input.position, 1.0);
    // New: pass-through selector, size, and rounding
    out.shape_selector = vertex_input.shape_selector;
    out.shape_size = vertex_input.shape_size;
    out.shape_data = vertex_input.shape_data;
    out.color = vertex_input.color;
    return out;
}


// Fragment shader

// v2:
// - version 1 would not render horizontal / vertical edges pixel perfect. Only with df_aa_factor 0.5, but 
//   then the diagonal anti-aliasing is too crisp.

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Clip fragments outside the clip rectangle (exclusive bounds)
    if (in.model_pos.x < pc.clip_rect_x.x || in.model_pos.x >= pc.clip_rect_x.y ||
        in.model_pos.y < pc.clip_rect_y.x || in.model_pos.y >= pc.clip_rect_y.y) {
        discard;
    }
    
    let distance = compute_distance(in);
    // Adaptive screen-space AA using intrinsic fwidth(distance)
    // fwidth(x) = abs(dfdx(x)) + abs(dfdy(x)); gives 1.0 on axis-aligned SDF edges, ~1.414 at 45°.
    let afwidth = fwidth(distance) * 0.5;
    let val = smoothstep(-afwidth, afwidth, distance);
    return vec4(in.color.rgb, in.color.a * val);
}

// v1

// Screen-space antialiasing factor (0.5 ≤ α ≤ √2)
const df_aa_factor = 0.65;

@fragment
fn fs_main_v1(in: VertexOutput) -> @location(0) vec4<f32> {
    let distance = compute_distance(in);
    let df = length(vec2<f32>(dpdx(distance), dpdy(distance)));
    let afwidth = df_aa_factor * df;
    let val = smoothstep(-afwidth, afwidth, distance);
    return vec4(in.color.rgb, in.color.a * val);
}


fn compute_distance(in: VertexOutput) -> f32 {
    let half_shape_size = in.shape_size * 0.5;
    let p_local = in.unorm_tex_coords - half_shape_size;
    var distance: f32 = 0.0;
    switch (in.shape_selector) {
        case 0u: { // filled rect
            distance = -sd_filled_rect(p_local, half_shape_size);
        }
        case 1u: { // rounded rect (shape_data.x = radius)
            let radius = in.shape_data[0];
            distance = -sd_rounded_rect(p_local, half_shape_size, radius);
        }
        case 2u: { // circle (min half extent)
            let r = min(half_shape_size.x, half_shape_size.y);
            distance = -sd_circle(p_local, r);
        }
        case 3u: { // ellipse (use half extents)
            distance = -sd_ellipse(p_local, half_shape_size);
        }
        case 4u: { // chamfer rect (shape_data.x = chamfer)
            let chamfer = in.shape_data.x;
            distance = -sd_chamfer_rect(p_local, half_shape_size, chamfer);
        }
        case 10u: { // rect stroke (shape_data.xy = stroke thickness)
            let stroke = in.shape_data;
            distance = -sd_stroked_rect(p_local, half_shape_size, stroke);
        }
        default: { // fallback
            distance = -sd_filled_rect(p_local, half_shape_size);
        }
    }
    return distance;
}

// New: signed distance to axis-aligned rectangle (negative inside)
fn sd_filled_rect(p: vec2<f32>, half_size: vec2<f32>) -> f32 {
    let d = abs(p) - half_size;
    return length(max(d, vec2(0.0, 0.0))) + min(max(d.x, d.y), 0.0);
}

fn sd_circle(p : vec2<f32>, r : f32) -> f32 {
    return length(p) - r;
}

// Approximate signed distance to an axis-aligned ellipse with radii r (half-size) centered at the origin.
// Negative inside, positive outside. Based on Inigo Quilez formula: https://iquilezles.org/articles/distfunctions2d/
fn sd_ellipse(p: vec2<f32>, r: vec2<f32>) -> f32 {
    // Avoid division by zero if any radius is zero (degenerates to line); clamp radii minimally.
    let rr = max(r, vec2<f32>(1e-5, 1e-5));
    let k0 = length(p / rr);
    let k1 = length(p / (rr * rr));
    return k0 * (k0 - 1.0) / k1;
}

// From <https://iquilezles.org/articles/distfunctions>
fn sd_rounded_rect(p: vec2<f32>, size: vec2<f32>, radius: f32) -> f32 {
    return length(max(abs(p) - size + vec2(radius, radius), vec2(0.0, 0.0)))-radius;
}

fn sd_stroked_rect(p: vec2<f32>, half_size: vec2<f32>, stroke: vec2<f32>) -> f32 {
    // Clamp inner half-size to non-negative
    let inner_half = max(half_size - stroke, vec2<f32>(0.0, 0.0));
    let d_outer = sd_filled_rect(p, half_size);
    let d_inner = sd_filled_rect(p, inner_half);
    // Ring = outer minus inner: max(d_outer, -d_inner)
    return max(d_outer, -d_inner);
}

// Chamfer rectangle SDF (based on Inigo Quilez sdChamferBox)
fn sd_chamfer_rect(p: vec2<f32>, half_size: vec2<f32>, chamfer: f32) -> f32 {
    // b is the inner box after removing chamfer extents
    let b = half_size - vec2<f32>(chamfer, chamfer);
    var q = abs(p) - b;
    if (q.y > q.x) {
        q = vec2<f32>(q.y, q.x);
    }
    q.y = q.y + chamfer;
    let k = 1.0 - sqrt(2.0);
    if (q.y < 0.0 && q.y + q.x * k < 0.0) {
        return q.x;
    }
    if (q.x < q.y) {
        return (q.x + q.y) * sqrt(0.5);
    }
    return length(q);
}
