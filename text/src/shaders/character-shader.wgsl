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

@group(1) @binding(0)
var t_texture: texture_2d<f32>;
@group(1) @binding(1)
var<uniform> texture_size: vec2<f32>;
@group(1) @binding(2)
var s_sampler: sampler;

@fragment
fn fs_flat(in: VertexOutput) -> @location(0) vec4<f32> {
    let sample = textureSample(t_texture, s_sampler, in.tex_coords);
    let alpha = sample.r;

    return vec4<f32>(0.0, 0.0, 0.0, alpha);
}

// For the fragment shader:
//   The distance field is constructed as unsigned char values,
//   so that the zero value is at 128, and the supported range of distances is [-4 * 127/128, 4].
//   Hence our multiplier (width of the range) is 4 * 255/128 and zero threshold is 128/255.
const df_multiplier = 7.96875;
const df_threshold = 0.50196078431;

// @fragment
// fn fs_sdf(in: VertexOutput) -> @location(0) vec4<f32> {
//     // fetch the SDF value from the texture
//     let distance = (textureSample(t_texture, s_sampler, in.tex_coords).r - 0.50196078431) * 7.96875;

//     // apply anti-aliasing

//     let width: f32 = length(vec2<f32>(dpdx(distance), dpdy(distance))) * 0.70710678118654757;
//     let alpha = smoothstep(0.0, width, distance);

//     return vec4<f32>(0.0, 0.0, 0.0, alpha);
// }

// Assuming a radius of a little less than the diagonal of the fragment
const df_aa_factor = 0.65;
const half_sqrt2 = 0.70710678118654757;
const df_epsilon = 0.0001;

@fragment
fn fs_sdf(in: VertexOutput) -> @location(0) vec4<f32> {
    // fetch the SDF value from the texture
    let distance = (textureSample(t_texture, s_sampler, in.tex_coords).r - df_threshold) * df_multiplier;

    // apply anti-aliasing

    var dist_grad: vec2<f32> = vec2(dpdx(distance), dpdy(distance));

    let dg_len2 : f32 = dot(dist_grad, dist_grad);

    if (dg_len2 < df_epsilon) {
        dist_grad = vec2(half_sqrt2, half_sqrt2);
    } else {
        dist_grad = dist_grad * inverseSqrt(dg_len2);
    };

    let unorm_text_coords = in.tex_coords * texture_size;

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

    return vec4<f32>(0.0, 0.0, 0.0, val);
}

