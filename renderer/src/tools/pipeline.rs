const VERTEX_SHADER_ENTRY: &str = "vs_main";

/// A consolidated set of parameters for the pipeline creation.
#[derive(Debug)]
pub struct PipelineParams {
    pub shader: wgpu::ShaderModule,
    pub pipeline_layout: wgpu::PipelineLayout,
    pub targets: [Option<wgpu::ColorTargetState>; 1],
    pub vertex_layout: [wgpu::VertexBufferLayout<'static>; 1],
}

impl PipelineParams {
    pub fn create_pipeline(
        &self,
        label: &str,
        device: &wgpu::Device,
        fragment_shader_entry: &str,
    ) -> wgpu::RenderPipeline {
        create_pipeline(
            label,
            device,
            &self.shader,
            fragment_shader_entry,
            &self.vertex_layout,
            &self.pipeline_layout,
            &self.targets,
        )
    }
}

pub fn create_pipeline(
    label: &str,
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    fragment_shader_entry: &str,
    vertex_layout: &[wgpu::VertexBufferLayout],
    pipeline_layout: &wgpu::PipelineLayout,
    targets: &[Option<wgpu::ColorTargetState>],
) -> wgpu::RenderPipeline {
    let pipeline = wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(VERTEX_SHADER_ENTRY),
            compilation_options: Default::default(),
            buffers: vertex_layout,
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_shader_entry),
            compilation_options: Default::default(),
            targets,
        }),
        primitive: wgpu::PrimitiveState {
            cull_mode: Some(wgpu::Face::Back),
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    };

    device.create_render_pipeline(&pipeline)
}
