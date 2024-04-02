// Vertex shader

@group(0) @binding(0)
var<uniform> model_view: mat4x4<f32>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

@vertex
fn vs_main(
    vertex_input: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coords = vertex_input.tex_coords;
    out.clip_position = model_view * vec4<f32>(vertex_input.position, 1.0);
    return out;
}

// Fragment shader

struct ShapeSize {
    value: vec2<f32>,
    _padding: vec2<f32>
}

@group(1) @binding(0)
var<uniform> shape_size: ShapeSize;

// For the fragment shader:
//   The distance field is constructed as unsigned char values,
//   so that the zero value is at 128, and the supported range of distances is [-4 * 127/128, 4].
//   Hence our multiplier (width of the range) is 4 * 255/128 and zero threshold is 128/255.
const df_multiplier = 7.96875;
const df_threshold = 0.50196078431;

// Assuming a radius of a little less than the diagonal of the fragment
const df_aa_factor = 0.65;
const half_sqrt2 = 0.70710678118654757;
const df_epsilon = 0.0001;

fn sd_circle(p : vec2<f32>, r : f32) -> f32 {
    return length(p) - r;
}

@fragment
fn fs_sdf_circle(in: VertexOutput) -> @location(0) vec4<f32> {
    let norm_tex_coords = in.tex_coords - vec2<f32>(0.5,0.5);

    var distance: f32 = sd_circle(norm_tex_coords, 0.5) * -1.0;

    // Multiplying by `shape_size` to correct the aspect ratio of the distance vector, so that
    // ellipses can be rendered with proper anti-aliasing.
    var dist_grad: vec2<f32> = vec2(dpdx(distance), dpdy(distance)) * shape_size.value;

    // Normalize distance gradient vector.
    let dg_len2 : f32 = dot(dist_grad, dist_grad);
    if (dg_len2 < df_epsilon) {
        dist_grad = vec2(half_sqrt2, half_sqrt2);
    } else {
        dist_grad = dist_grad * inverseSqrt(dg_len2);
    };

    // let unorm_text_coords = norm_tex_coords;
    let unorm_text_coords = in.tex_coords;

    let jdx = dpdx(unorm_text_coords);
    let jdy = dpdy(unorm_text_coords);

    let grad = vec2(dot(dist_grad, jdx), dot(dist_grad, jdy));

    let afwidth = df_aa_factor * length(grad);

    // gamma correct
    // let val = saturate((distance + afwidth) / (2.0 * afwidth));
    let val = smoothstep(-afwidth, afwidth, distance);

    return vec4<f32>(0.0, 0.0, 0.0, val);
}

// From <https://iquilezles.org/articles/distfunctions>
fn sd_rounded_rect(p: vec2<f32>, size: vec2<f32>, radius: f32) -> f32 {
    return length(max(abs(p) - size + vec2(radius, radius), vec2(0.0, 0.0)))-radius;
}

@fragment
fn fs_sdf_rounded_rect(in: VertexOutput) -> @location(0) vec4<f32> {
    let norm_tex_coords = in.tex_coords - vec2<f32>(0.5,0.5);

    let half_shape_size = shape_size.value * 0.5;

    var distance: f32 = sd_rounded_rect(norm_tex_coords * shape_size.value, half_shape_size * 0.5, 10.0) * -1.0 ;
    var dist_grad: vec2<f32> = vec2(dpdx(distance), dpdy(distance));

    // Normalize distance gradient vector.
    let dg_len2 : f32 = dot(dist_grad, dist_grad);
    if (dg_len2 < df_epsilon) {
        dist_grad = vec2(half_sqrt2, half_sqrt2);
    } else {
        dist_grad = dist_grad * inverseSqrt(dg_len2);
    };

    let unorm_text_coords = in.tex_coords * shape_size.value;

    let jdx = dpdx(unorm_text_coords);
    let jdy = dpdy(unorm_text_coords);

    let grad = vec2(dot(dist_grad, jdx), dot(dist_grad, jdy));

    let afwidth = df_aa_factor * length(grad);

    // gamma correct
    // let val = saturate((distance + afwidth) / (2.0 * afwidth));
    let val = smoothstep(-afwidth, afwidth, distance);

    return vec4<f32>(0.0, 0.0, 0.0, val);
}
