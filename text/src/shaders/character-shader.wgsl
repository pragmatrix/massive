// Vertex shader

@group(1) @binding(0)
var<uniform> camera: mat4x4<f32>;
@group(2) @binding(0)
var<uniform> model: mat4x4<f32>;

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
    // out.clip_position = camera * model * vec4<f32>(vertex_input.position, 1.0);
    out.clip_position = model * vec4<f32>(vertex_input.position, 1.0);
    return out;
}

// Fragment shader

@group(0) @binding(0)
var t_texture: texture_2d<f32>;
@group(0) @binding(1)
var s_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let sample = textureSample(t_texture, s_sampler, in.tex_coords);
    let alpha = sample.r;

    return vec4<f32>(0.0, 0.0, 0.0, alpha);
}
