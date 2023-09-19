pub struct BindGroupLayoutBuilder {
    shader_stages: wgpu::ShaderStages,
    entries: Vec<wgpu::BindGroupLayoutEntry>,
}

impl BindGroupLayoutBuilder {
    pub fn vertex() -> Self {
        Self::new(wgpu::ShaderStages::VERTEX)
    }

    pub fn fragment() -> Self {
        Self::new(wgpu::ShaderStages::FRAGMENT)
    }

    fn new(shader_stages: wgpu::ShaderStages) -> Self {
        Self {
            shader_stages,
            entries: Vec::new(),
        }
    }

    pub fn uniform(self) -> Self {
        self.add_ty(wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        })
    }

    pub fn texture(self) -> Self {
        self.add_ty(wgpu::BindingType::Texture {
            multisampled: false,
            view_dimension: wgpu::TextureViewDimension::D2,
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
        })
    }

    pub fn sampler(self) -> Self {
        self.add_ty(wgpu::BindingType::Sampler(
            wgpu::SamplerBindingType::Filtering,
        ))
    }

    fn add_ty(mut self, ty: wgpu::BindingType) -> Self {
        self.entries.push(wgpu::BindGroupLayoutEntry {
            binding: self.entries.len() as _,
            visibility: self.shader_stages,
            ty,
            count: None,
        });
        self
    }

    pub fn build(self, name: &str, device: &wgpu::Device) -> wgpu::BindGroupLayout {
        let descriptor = wgpu::BindGroupLayoutDescriptor {
            label: Some(name),
            entries: &self.entries,
        };
        device.create_bind_group_layout(&descriptor)
    }
}
