// Vertex shader (instanced quads)

var<push_constant> view_model: mat4x4<f32>;

struct VertexInput {
    @builtin(vertex_index) vertex_index: u32,
    // Position rectangle in pixels: left-top and right-bottom
    @location(0) pos_lt: vec2<f32>,
    @location(1) pos_rb: vec2<f32>,
    // Texture rect in atlas pixel space: left-top and right-bottom
    @location(2) uv_lt: vec2<f32>,
    @location(3) uv_rb: vec2<f32>,
    // Depth in pixel space
    @location(4) depth: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    // Unnormalized texture pixel coordinates.
    @location(0) tex_coords: vec2<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
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
    return out;
}

// Fragment shader

@group(0) @binding(0)
var t_texture: texture_2d<f32>;
@group(0) @binding(1)
var s_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let texture_size = vec2<f32>(textureDimensions(t_texture));
    return textureSample(t_texture, s_sampler, in.tex_coords / texture_size);
}
