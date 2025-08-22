// Vertex shader

var<push_constant> view_model: mat4x4<f32>;

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
}

@vertex
fn vs_main(
    vertex_input: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.unorm_tex_coords = vertex_input.unorm_tex_coords;
    out.clip_position = view_model * vec4<f32>(vertex_input.position, 1.0);
    // New: pass-through selector, size, and rounding
    out.shape_selector = vertex_input.shape_selector;
    out.shape_size = vertex_input.shape_size;
    out.shape_data = vertex_input.shape_data;
    out.color = vertex_input.color;
    return out;
}

// Fragment shader

// Screen-space antialiasing factor
const df_aa_factor = 0.65;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Common precomputations
    let half_shape_size = in.shape_size * 0.5;
    let p_local = in.unorm_tex_coords - half_shape_size; // centered, pixel-space

    var distance: f32;

    switch (in.shape_selector) {
        case 0u: {
            // Filled rect
            distance = -sd_filled_rect(p_local, half_shape_size);
        }
        case 1u: {
            // Rounded rect
            let radius = in.shape_data[0];
            distance = -sd_rounded_rect(p_local, half_shape_size, radius);
        }
        case 2u: {
            // Circle
            let r = min(half_shape_size.x, half_shape_size.y);
            distance = -sd_circle(p_local, r);
        }
        case 3u: {
            // Ellipse using half_shape_size as radii
            distance = -sd_ellipse(p_local, half_shape_size);
        }
        case 10u: {
            // Rect stroke: shape_data.xy = per-axis stroke thickness (x for vertical edges, y for horizontal edges thickness)
            let stroke = in.shape_data;
            distance = -sd_stroked_rect(p_local, half_shape_size, stroke);
        }

        default: {
            // Filled rect
            distance = -sd_filled_rect(p_local, half_shape_size);
        }
    }

    // Screen-space AA using distance derivatives
    let df = length(vec2<f32>(dpdx(distance), dpdy(distance)));
    let afwidth = df_aa_factor * df;
    let val = smoothstep(-afwidth, afwidth, distance);

    // Straight-alpha output: modulate only alpha by coverage
    return vec4(in.color.rgb, in.color.a * val);
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
