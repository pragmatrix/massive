// Vertex shader

struct PushConstants {
    view_model: mat4x4<f32>,
    clip_rect_x: vec2<f32>, // [min_x, max_x]
    clip_rect_y: vec2<f32>, // [min_y, max_y]
}

var<push_constant> pc: PushConstants;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    // Unnormalized texture pixel coordinates.
    @location(0) tex_coords: vec2<f32>,
    @location(1) model_pos: vec2<f32>,
}

@vertex
fn vs_main(
    vertex_input: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coords = vertex_input.tex_coords;
    out.model_pos = vertex_input.position.xy;
    out.clip_position = pc.view_model * vec4<f32>(vertex_input.position, 1.0);
    return out;
}

// Fragment shader

@group(0) @binding(0)
var t_texture: texture_2d<f32>;
@group(0) @binding(1)
var s_sampler: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Clip fragments outside the clip rectangle (exclusive bounds)
    if (in.model_pos.x < pc.clip_rect_x.x || in.model_pos.x >= pc.clip_rect_x.y ||
        in.model_pos.y < pc.clip_rect_y.x || in.model_pos.y >= pc.clip_rect_y.y) {
        discard;
    }
    
    let texture_size = vec2<f32>(textureDimensions(t_texture));
    return textureSample(t_texture, s_sampler, in.tex_coords / texture_size);
}
