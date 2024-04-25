use wgpu::{AddressMode, Device, FilterMode, Sampler, SamplerDescriptor};

/// Creates a linear and edge clamping texture sampler.
///
/// This assumes that the underlying texture is padded.
pub fn linear_clamping(device: &Device) -> Sampler {
    device.create_sampler(&SamplerDescriptor {
        label: Some("Linear / Clamping Texture Sampler"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..Default::default()
    })
}
