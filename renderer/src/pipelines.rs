use crate::{
    pods::{TextureColorVertex, TextureVertex, VertexLayout},
    primitives::Pipeline,
    texture,
    tools::create_pipeline,
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

    let texture_vertex_layout = [TextureVertex::layout().clone()];
    let text_layer_vertex_layout = [TextureColorVertex::layout().clone()];

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
