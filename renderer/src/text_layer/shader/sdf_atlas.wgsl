// Vertex shader (instanced quads)

var<push_constant> view_model: mat4x4<f32>;

struct VertexInput {
    @builtin(vertex_index) vertex_index: u32,
    // Instance attributes
    // Position rectangle in pixels: left-top and right-bottom
    @location(0) pos_lt: vec2<f32>,
    @location(1) pos_rb: vec2<f32>,
    // Texture rect in atlas pixel space: left-top and right-bottom
    @location(2) uv_lt: vec2<f32>,
    @location(3) uv_rb: vec2<f32>,
    // Per-glyph color
    @location(4) color: vec4<f32>,
    // Depth in pixel space
    @location(5) depth: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    // Unnormalized texture pixel coordinates.
    @location(0) tex_coords: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    // Compute quad corner (0..3) from vertex_index & 3 for indexed draw [0,1,2,0,2,3]
    let i = input.vertex_index & 3u;
    let use_right = (i == 2u) || (i == 3u);
    let use_bottom = (i == 1u) || (i == 2u);

    let x = select(input.pos_lt.x, input.pos_rb.x, use_right);
    let y = select(input.pos_lt.y, input.pos_rb.y, use_bottom);
    let pos = vec3<f32>(x, y, input.depth);

    let tu = select(input.uv_lt.x, input.uv_rb.x, use_right);
    let tv = select(input.uv_lt.y, input.uv_rb.y, use_bottom);

    var out: VertexOutput;
    out.tex_coords = vec2<f32>(tu, tv);
    out.clip_position = view_model * vec4<f32>(pos, 1.0);
    out.color = input.color;
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
const df_aa_factor = 0.65;
const half_sqrt2 = 0.70710678118654757;
const df_epsilon = 0.0001;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
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

    return vec4<f32>(in.color.rgb, in.color.a * val);
}
