use bytemuck::{Pod, Zeroable};
use massive_geometry::Point3;
use std::mem;

// We need this for Rust to store our data correctly for the shaders
#[repr(C)]
// This is so we can store this in a buffer
#[derive(Debug, Copy, Clone, Pod, Zeroable)]
pub struct Matrix4(pub [[f32; 4]; 4]);

#[repr(C)]
// This is so we can store this in a buffer. Also adds padding for webgl.
#[derive(Debug, Copy, Clone, Pod, Zeroable)]
pub struct TextureSize(pub [f32; 2], pub [u32; 2]);

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    position: [f32; 3],
}

impl Vertex {
    fn new(x: f32, y: f32, z: f32) -> Self {
        Self {
            position: [x, y, z],
        }
    }

    #[allow(unused)]
    fn desc() -> &'static wgpu::VertexBufferLayout<'static> {
        const LAYOUT: wgpu::VertexBufferLayout = wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3],
        };

        &LAYOUT
    }
}

impl From<(f32, f32, f32)> for Vertex {
    fn from(v: (f32, f32, f32)) -> Self {
        Self::new(v.0, v.1, v.2)
    }
}

impl From<Point3> for Vertex {
    fn from(v: Point3) -> Self {
        let v = v.cast::<f32>().expect("Failed to cast Point3 to f32");
        Self::new(v.x, v.y, v.z)
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct TextureVertex {
    pub position: Vertex,
    pub tex_coords: [f32; 2],
}

impl TextureVertex {
    pub fn desc() -> &'static wgpu::VertexBufferLayout<'static> {
        const LAYOUT: wgpu::VertexBufferLayout = wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<TextureVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2],
        };

        &LAYOUT
    }
}
