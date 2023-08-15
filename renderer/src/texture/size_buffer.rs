use wgpu::util::DeviceExt;

#[derive(Debug)]
pub struct SizeBuffer(wgpu::Buffer);

impl SizeBuffer {
    pub fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Texture Size Buffer"),
            contents: bytemuck::cast_slice(&[size.0, size.1]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Self(buffer)
    }

    pub fn as_binding(&self) -> wgpu::BindingResource {
        self.0.as_entire_binding()
    }
}
