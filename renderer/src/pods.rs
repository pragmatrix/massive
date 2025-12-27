use std::{
    fmt,
    mem::{self, size_of},
};

use bytemuck::{Pod, Zeroable};
use static_assertions::const_assert_eq;

use massive_geometry::Vector3;
use wgpu::{BufferAddress, VertexAttribute, VertexBufferLayout, VertexStepMode};

// We need this for Rust to store our data correctly for the shaders
#[repr(C)]
// This is so we can store this in a buffer
#[derive(Debug, Copy, Clone, Pod, Zeroable)]
pub struct Matrix4(pub [[f32; 4]; 4]);

// WebGL uniform requirement
const_assert_eq!(size_of::<Matrix4>() % 16, 0);

/// Clip rectangle in model space with exclusive bounds.
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod, Zeroable)]
pub struct ClipRect {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl ClipRect {
    /// Clip rectangle that effectively disables clipping.
    pub const NONE: Self = Self {
        min_x: f32::MIN,
        min_y: f32::MIN,
        max_x: f32::MAX,
        max_y: f32::MAX,
    };

    #[allow(unused)]
    pub fn new(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Self {
        Self {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }
}

/// Push constants for rendering, containing view-model matrix and clip rectangle.
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod, Zeroable)]
pub struct PushConstants {
    pub view_model: Matrix4,
    pub clip_rect: ClipRect,
}

// WebGL uniform requirement
const_assert_eq!(size_of::<PushConstants>() % 16, 0);

#[repr(C)]
#[derive(Debug, Copy, Clone, Pod, Zeroable)]
pub struct TextureSize(pub [f32; 2], pub [u32; 2]);

// WebGL uniform requirement
const_assert_eq!(size_of::<TextureSize>() % 16, 0);

/// RGBA color
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod, Zeroable)]
pub struct Color(pub [f32; 4]);

// WebGL uniform requirement
const_assert_eq!(size_of::<Color>() % 16, 0);

impl From<massive_geometry::Color> for Color {
    fn from(value: massive_geometry::Color) -> Self {
        Self([value.red, value.green, value.blue, value.alpha])
    }
}

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
}

impl VertexLayout for Vertex {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3],
        }
    }
}

impl From<(f32, f32, f32)> for Vertex {
    fn from(v: (f32, f32, f32)) -> Self {
        Self::new(v.0, v.1, v.2)
    }
}

impl From<Vector3> for Vertex {
    fn from(v: Vector3) -> Self {
        let v = v.as_vec3();
        Self::new(v.x, v.y, v.z)
    }
}

#[allow(unused)]
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ColorVertex {
    pub position: Vertex,
    pub color: Color,
}

#[allow(unused)]
impl ColorVertex {
    pub fn new(position: impl Into<Vertex>, color: impl Into<Color>) -> Self {
        Self {
            position: position.into(),
            color: color.into(),
        }
    }
}

impl VertexLayout for ColorVertex {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRS: [VertexAttribute; 2] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];

        VertexBufferLayout {
            array_stride: size_of::<ColorVertex>() as BufferAddress,
            step_mode: VertexStepMode::Vertex,
            attributes: &ATTRS,
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, Pod, Zeroable)]
pub struct Color3(pub [f32; 3]);

impl From<massive_geometry::Color> for Color3 {
    fn from(value: massive_geometry::Color) -> Self {
        Self([value.red, value.green, value.blue])
    }
}

/// A color per instance, used in the TextLayer
/// TODO: this does not belong here. Move these into `text_layer/``
/// TODO: May remove, it's not used anymore.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct InstanceColor {
    pub color: [f32; 3],
}

impl VertexLayout for InstanceColor {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRS: [VertexAttribute; 1] = wgpu::vertex_attr_array![2 => Float32x3];

        VertexBufferLayout {
            array_stride: size_of::<InstanceColor>() as BufferAddress,
            step_mode: VertexStepMode::Instance,
            attributes: &ATTRS,
        }
    }
}

impl From<massive_geometry::Color> for InstanceColor {
    fn from(value: massive_geometry::Color) -> Self {
        Self {
            color: [value.red, value.green, value.blue],
        }
    }
}

// Some helpful traits.

pub trait VertexLayout {
    fn layout() -> wgpu::VertexBufferLayout<'static>;
}

pub trait ToPod {
    type Pod;
    fn to_pod(&self) -> Self::Pod;
}

pub trait AsBytes {
    fn as_bytes(&self) -> &[u8];
    fn size<R: TryFrom<usize>>() -> R
    where
        R::Error: fmt::Debug;
}

impl<T: Pod> AsBytes for T {
    fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }

    fn size<R: TryFrom<usize>>() -> R
    where
        R::Error: fmt::Debug,
    {
        mem::size_of::<Self>()
            .try_into()
            .expect("Failed to convert usize to the required size type")
    }
}

pub mod glam_impl {
    use crate::pods;

    use super::ToPod;
    use glam::DMat4;

    impl ToPod for DMat4 {
        type Pod = super::Matrix4;

        fn to_pod(&self) -> Self::Pod {
            let m = self.as_mat4();
            pods::Matrix4(m.to_cols_array_2d())
        }
    }
}
