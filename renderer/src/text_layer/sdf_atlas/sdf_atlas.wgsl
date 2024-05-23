// Vertex shader

@group(0) @binding(0)
var<uniform> view_model: mat4x4<f32>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) color: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    // Unnormalized texture pixel coordinates.
    @location(0) tex_coords: vec2<f32>,
    @location(1) @interpolate(flat) color: vec3<f32>,
}

@vertex
fn vs_main(
    vertex_input: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coords = vertex_input.tex_coords;
    out.clip_position = view_model * vec4<f32>(vertex_input.position, 1.0);
    out.color = vertex_input.color;
    return out;
}

// Fragment shader

struct TextureSize {
    value: vec2<f32>,
    _padding: vec2<f32>,
}

@group(1) @binding(0)
var t_texture: texture_2d<f32>;
@group(1) @binding(1)
var s_sampler: sampler;

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

@fragment
fn fs_sdf(in: VertexOutput) -> @location(0) vec4<f32> {
    // fetch the SDF value from the texture
    // OO: Use 1 / texture_size and multiply.
    let texture_size = vec2<f32>(textureDimensions(t_texture));
    let distance = (textureSample(t_texture, s_sampler, in.tex_coords / texture_size).r - df_threshold) * df_multiplier;

    // apply anti-aliasing
    var dist_grad: vec2<f32> = vec2(dpdx(distance), dpdy(distance));

    let dg_len2 : f32 = dot(dist_grad, dist_grad);

    if (dg_len2 < df_epsilon) {
        dist_grad = vec2(half_sqrt2, half_sqrt2);
    } else {
        dist_grad = dist_grad * inverseSqrt(dg_len2);
    };

    let unorm_text_coords = in.tex_coords;

    let jdx = dpdx(unorm_text_coords);
    let jdy = dpdy(unorm_text_coords);

    let grad = vec2(
        dist_grad.x*jdx.x + dist_grad.y*jdy.x,
        dist_grad.x*jdx.y + dist_grad.y*jdy.y
    );

    // let afwidth = length(grad) * half_sqrt2;
    let afwidth = df_aa_factor * length(grad);

    // gamma correct
    // let val = saturate((distance + afwidth) / (2.0 * afwidth));
    let val = smoothstep(-afwidth, afwidth, distance);

    return vec4<f32>(in.color, val);
}
