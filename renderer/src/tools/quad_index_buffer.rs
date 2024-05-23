use std::mem::size_of_val;

use wgpu::util::DeviceExt;

#[derive(Debug, derive_more::Deref)]
pub struct QuadIndexBuffer(wgpu::Buffer);

impl QuadIndexBuffer {
    pub fn new(device: &wgpu::Device) -> Self {
        // OO: Provide a good initial size.
        const NO_INDICES: [u16; 0] = [];
        Self(Self::create_buffer(device, &NO_INDICES))
    }

    pub fn quads(&self) -> usize {
        (self.0.size() as usize) / size_of_val(Self::QUAD_INDICES)
    }

    pub fn ensure_can_index_num_quads(
        &mut self,
        device: &wgpu::Device,
        required_quad_count: usize,
    ) {
        let current = self.quads();
        if required_quad_count <= current {
            return;
        }

        let mut proposed_quad_capacity = current.max(1) << 1;
        loop {
            if proposed_quad_capacity >= required_quad_count {
                break;
            }
            proposed_quad_capacity <<= 1;
            assert!(proposed_quad_capacity != 0);
        }

        log::debug!("Growing index buffer from {current} to {proposed_quad_capacity} quads, required: {required_quad_count}");

        let indices = Self::generate_array(self, proposed_quad_capacity);
        self.0 = Self::create_buffer(device, &indices);
    }

    fn generate_array(&self, quads: usize) -> Vec<u16> {
        let mut v = Vec::with_capacity(Self::QUAD_INDICES.len() * quads);

        (0..quads).for_each(|quad_index| {
            v.extend(
                Self::QUAD_INDICES
                    .iter()
                    .map(|i| *i + (quad_index << 2) as u16),
            )
        });

        v
    }

    pub const QUAD_INDICES: &'static [u16] = &[0, 1, 2, 0, 2, 3];
    pub const INDICES_PER_QUAD: usize = Self::QUAD_INDICES.len();

    fn create_buffer(device: &wgpu::Device, indices: &[u16]) -> wgpu::Buffer {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Quad Index Buffer"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        })
    }
}
