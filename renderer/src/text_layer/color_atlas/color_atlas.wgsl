// Vertex shader

@group(0) @binding(0)
var<uniform> view_model: mat4x4<f32>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    // Unnormalized texture pixel coordinates.
    @location(0) tex_coords: vec2<f32>,
}

@vertex
fn vs_main(
    vertex_input: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coords = vertex_input.tex_coords;
    out.clip_position = view_model * vec4<f32>(vertex_input.position, 1.0);
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
var<uniform> texture_size: TextureSize;
@group(1) @binding(2)
var s_sampler: sampler;

@fragment
fn fs_color(in: VertexOutput) -> @location(0) vec4<f32> {
    // OO: Use 1 / texture_size and multiply.
    return textureSample(t_texture, s_sampler, in.tex_coords / texture_size.value);
}
