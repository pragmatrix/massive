// Vertex shader

var<push_constant> view_model: mat4x4<f32>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    // Unnormalized texture coordinates.
    @location(1) unorm_tex_coords: vec2<f32>,
    @location(2) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    // Unnormalized texture pixel coordinates.
    @location(0) unorm_tex_coords: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
}

@vertex
fn vs_main(
    vertex_input: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.unorm_tex_coords = vertex_input.unorm_tex_coords;
    out.clip_position = view_model * vec4<f32>(vertex_input.position, 1.0);
    out.color = vertex_input.color;
    return out;
}

// Fragment shader

@group(0) @binding(0)
var t_texture: texture_2d<f32>;
@group(0) @binding(1)
var s_sampler: sampler;

// For the fragment shader:
//   The distance field is constructed as unsigned char values,
//   so that the zero value is at 128, and the supported range of distances is [-4 * 127/128, 4].
//   Hence our multiplier (width of the range) is 4 * 255/128 and zero threshold is 128/255.
const df_multiplier = 7.96875;
const df_threshold = 0.50196078431;

// Assuming a radius of a little less than the diagonal of the fragment

// GPT-5: 
//
// For a 1×1 px box filter, the AA half‑width along a unit direction n = (nx, ny) is 0.5(|nx| + |ny|).
// This varies with edge angle: 0.5 along axes, up to √2/2 ≈ 0.707 at 45°.
//
// Using a single constant can’t match all angles. 0.65 is a practical compromise:
// It’s close to the angular average 2/π ≈ 0.6366 of 0.5(|nx|+|ny|).
// It compensates a bit for extra softening from bilinear sampling, mip/filtering, SDF quantization, and smoothstep’s shape.
const df_aa_factor = 0.65;

const half_sqrt2 = 0.70710678118654757;
const df_epsilon = 0.0001;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // fetch the SDF value from the texture
    // OO: Use 1 / texture_size and multiply.
    let texture_size = vec2<f32>(textureDimensions(t_texture));
    let norm_tex_coords = in.unorm_tex_coords / texture_size;
    let distance = (textureSample(t_texture, s_sampler, norm_tex_coords)[0] - df_threshold) * df_multiplier;

    // Compute the unit edge normal vector in screen space.
    var dist_grad = vec2(dpdx(distance), dpdy(distance));

    let dg_len2 : f32 = dot(dist_grad, dist_grad);

    if (dg_len2 < df_epsilon) {
        dist_grad = vec2(half_sqrt2, half_sqrt2);
    } else {
        dist_grad = dist_grad * inverseSqrt(dg_len2);
    };

    // Jacobian matrix that maps screen space to texture space.
    let unorm_tex_coords = in.unorm_tex_coords;

    let jdx = dpdx(unorm_tex_coords);
    let jdy = dpdy(unorm_tex_coords);

    // Transform the screen space unit vector into the texture space.
    let grad = vec2(
        dist_grad.x*jdx.x + dist_grad.y*jdy.x,
        dist_grad.x*jdx.y + dist_grad.y*jdy.y
    );

    // The number of texels traversed when moving one screen pixel along the edge normal.
    let texels_moved_in_normal_dir = length(grad);

    let af_width = texels_moved_in_normal_dir * df_aa_factor;

    // gamma correct
    // let val = saturate((distance + afwidth) / (2.0 * afwidth));
    let val = smoothstep(-af_width, af_width, distance);

    return vec4<f32>(in.color.rgb, in.color.a * val);
}
