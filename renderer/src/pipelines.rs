use crate::{
    bind_group_entries,
    pods::{TextureColorVertex, TextureVertex},
    primitives::Pipeline,
    texture,
    tools::{BindGroupLayoutBuilder, create_pipeline},
};

#[allow(unused)]
pub fn create(
    device: &wgpu::Device,
    view_projection_bind_group_layout: &wgpu::BindGroupLayout,
    texture_bind_group_layout: &texture::BindGroupLayout,
    shape_bind_group_layout: &wgpu::BindGroupLayout,
    targets: &[Option<wgpu::ColorTargetState>],
) -> Vec<(Pipeline, wgpu::RenderPipeline)> {
    let glyph_shader = &device.create_shader_module(wgpu::include_wgsl!("texture/glyph.wgsl"));

    let glyph_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Glyph Pipeline Layout"),
        bind_group_layouts: &[view_projection_bind_group_layout, texture_bind_group_layout],
        push_constant_ranges: &[],
    });

    let shape_shader = &device.create_shader_module(wgpu::include_wgsl!("shape/shape.wgsl"));

    let shape_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Shape Pipeline Layout"),
        bind_group_layouts: &[view_projection_bind_group_layout, shape_bind_group_layout],
        push_constant_ranges: &[],
    });

    let texture_vertex_layout = [TextureVertex::layout()];
    let text_layer_vertex_layout = [TextureColorVertex::layout()];

    [
        (
            Pipeline::PlanarGlyph,
            create_pipeline(
                "Planar Glyph Pipeline",
                device,
                glyph_shader,
                "fs_planar",
                &texture_vertex_layout,
                &glyph_pipeline_layout,
                targets,
            ),
        ),
        (
            Pipeline::SdfGlyph,
            create_pipeline(
                "SDF Glyph Pipeline",
                device,
                glyph_shader,
                "fs_sdf_glyph",
                &texture_vertex_layout,
                &glyph_pipeline_layout,
                targets,
            ),
        ),
        (
            Pipeline::Circle,
            create_pipeline(
                "Circle Pipeline",
                device,
                shape_shader,
                "fs_sdf_circle",
                &texture_vertex_layout,
                &shape_pipeline_layout,
                targets,
            ),
        ),
        (
            Pipeline::RoundedRect,
            create_pipeline(
                "Rounded Rect Pipeline",
                device,
                shape_shader,
                "fs_sdf_rounded_rect",
                &texture_vertex_layout,
                &shape_pipeline_layout,
                targets,
            ),
        ),
    ]
    .into()
}

pub fn create_view_projection_bind_group(
    device: &wgpu::Device,
    view_projection_buffer: &wgpu::Buffer,
) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
    let layout = BindGroupLayoutBuilder::vertex_stage()
        .uniform()
        .build("View Projection Bind Group Layout", device);

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &layout,
        entries: bind_group_entries!(0 => view_projection_buffer),
        label: Some("Camera Bind Group"),
    });

    (layout, bind_group)
}
