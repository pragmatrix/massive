// Vertex shader

@group(0) @binding(0)
var<uniform> model_view: mat4x4<f32>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(2) color: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
}

@vertex
fn vs_main(
    vertex_input: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = model_view * vec4<f32>(vertex_input.position, 1.0);
    out.color = vertex_input.color;
    return out;
}

// Fragment shader

@fragment
fn fs_simple(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}