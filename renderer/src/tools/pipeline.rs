const VERTEX_SHADER_ENTRY: &str = "vs_main";
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
// With LessEqual depth compare, negative constant bias pulls decals toward the camera.
const DECAL_DEPTH_BIAS_CONSTANT: i32 = -1;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PipelineVariant {
    Standard,
    Decal,
}

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
        variant: PipelineVariant,
    ) -> wgpu::RenderPipeline {
        create_pipeline(
            label,
            device,
            &self.shader,
            fragment_shader_entry,
            &self.vertex_layout,
            &self.pipeline_layout,
            &self.targets,
            variant,
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub fn create_pipeline(
    label: &str,
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    fragment_shader_entry: &str,
    vertex_layout: &[wgpu::VertexBufferLayout],
    pipeline_layout: &wgpu::PipelineLayout,
    targets: &[Option<wgpu::ColorTargetState>],
    variant: PipelineVariant,
) -> wgpu::RenderPipeline {
    let label = variant_label(label, variant);

    let pipeline = wgpu::RenderPipelineDescriptor {
        label: Some(&label),
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
        depth_stencil: Some(depth_stencil_state(variant)),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    };

    device.create_render_pipeline(&pipeline)
}

fn variant_label(base_label: &str, variant: PipelineVariant) -> String {
    match variant {
        PipelineVariant::Standard => base_label.to_owned(),
        PipelineVariant::Decal => format!("{base_label} Decal"),
    }
}

fn depth_stencil_state(variant: PipelineVariant) -> wgpu::DepthStencilState {
    match variant {
        PipelineVariant::Standard => wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        },
        PipelineVariant::Decal => wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState {
                constant: DECAL_DEPTH_BIAS_CONSTANT,
                slope_scale: 0.0,
                clamp: 0.0,
            },
        },
    }
}
