use std::mem::{self, size_of_val};

use log::debug;
use wgpu::{util::DeviceExt, BufferSlice, IndexFormat, RenderPass};

#[derive(Debug)]
pub struct QuadIndexBuffer(wgpu::Buffer);

type Index = u32;

impl QuadIndexBuffer {
    // OO: Use only 16 bit if not more is needed.
    pub const INDEX_FORMAT: IndexFormat = IndexFormat::Uint32;

    pub fn new(device: &wgpu::Device) -> Self {
        // OO: Provide a good initial size.
        const NO_INDICES: [Index; 0] = [];
        Self(Self::create_buffer(device, &NO_INDICES))
    }

    pub fn quads(&self) -> usize {
        (self.0.size() as usize) / size_of_val(Self::QUAD_INDICES)
    }

    pub fn set<'a, 'rpass>(&'a self, pass: &mut RenderPass<'rpass>, max_quads: Option<usize>)
    where
        'a: 'rpass,
    {
        let slice = {
            match max_quads {
                Some(max_quads) => self.slice(max_quads),
                None => self.0.slice(..),
            }
        };

        pass.set_index_buffer(slice, Self::INDEX_FORMAT)
    }

    fn slice(&self, max_quads: usize) -> BufferSlice {
        self.0
            .slice(..(max_quads * Self::INDICES_PER_QUAD * Self::INDEX_SIZE) as u64)
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

        debug!("Growing index buffer from {current} to {proposed_quad_capacity} quads, required: {required_quad_count}");

        let indices = Self::generate_array(self, proposed_quad_capacity);
        self.0 = Self::create_buffer(device, &indices);
    }

    fn generate_array(&self, quads: usize) -> Vec<Index> {
        let mut v = Vec::with_capacity(Self::QUAD_INDICES.len() * quads);

        (0..quads).for_each(|quad_index| {
            let offset = quad_index * Self::VERTICES_PER_QUAD;
            v.extend(Self::QUAD_INDICES.iter().map(|i| *i + offset as Index))
        });

        v
    }

    pub const QUAD_INDICES: &'static [Index] = &[0, 1, 2, 0, 2, 3];
    pub const INDICES_PER_QUAD: usize = Self::QUAD_INDICES.len();
    pub const VERTICES_PER_QUAD: usize = 4;
    const INDEX_SIZE: usize = mem::size_of::<Index>();

    fn create_buffer(device: &wgpu::Device, indices: &[Index]) -> wgpu::Buffer {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Quad Index Buffer"),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        })
    }
}
