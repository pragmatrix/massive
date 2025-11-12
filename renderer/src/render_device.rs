use anyhow::{Context, Result, bail};
use log::info;

const REQUIRED_ADAPTER_FEATURES: wgpu::Features = wgpu::Features::PUSH_CONSTANTS;

#[derive(Debug, Clone)]
pub struct RenderDevice {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface_format: wgpu::TextureFormat,
    pub alpha_mode: wgpu::CompositeAlphaMode,
}

impl RenderDevice {
    pub async fn for_surface(
        instance: wgpu::Instance,
        surface: &wgpu::Surface<'static>,
    ) -> Result<Self> {
        let adapter = get_adapter_for_surface(instance, surface).await?;

        info!("GPU Adapter backend: {:?}", adapter.get_info().backend);
        let surface_caps = surface.get_capabilities(&adapter);
        // Don't use srgb now, colors are specified in linear rgb space.
        let surface_format = *surface_caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .unwrap_or(&surface_caps.formats[0]);

        info!("- Surface format: {surface_format:?}");

        info!(
            "- Available present modes: {:?}",
            surface_caps.present_modes
        );
        // Robustness: Select the alpha mode explicitly?
        let alpha_mode = surface_caps.alpha_modes[0];
        info!("- Selected alpha mode: {alpha_mode:?}");

        let (device, queue) = get_device_and_queue_from_adapter(adapter).await?;

        info!(
            "- Max texture dimension: {}",
            device.limits().max_texture_dimension_2d
        );

        Ok(Self {
            device,
            queue,
            surface_format,
            alpha_mode,
        })
    }
}

async fn get_device_and_queue_from_adapter(
    adapter: wgpu::Adapter,
) -> Result<(wgpu::Device, wgpu::Queue)> {
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            required_features: REQUIRED_ADAPTER_FEATURES,
            // May be wrong, see: <https://github.com/gfx-rs/wgpu/blob/1144b065c4784d769d59da2f58f5aa13212627b0/examples/src/hello_triangle/mod.rs#L33-L34>
            required_limits: adapter.limits(),
            label: None,
            memory_hints: Default::default(),
            trace: Default::default(),
        })
        .await
        .context("Requesting device")
}

async fn get_adapter_for_surface(
    instance: wgpu::Instance,
    surface: &wgpu::Surface<'static>,
) -> Result<wgpu::Adapter> {
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            // Be sure the adapter can present the surface.
            compatible_surface: Some(surface),
            // software fallback?
            force_fallback_adapter: false,
        })
        .await
        .context("GPU Adapter not found")?;

    if !adapter.features().contains(REQUIRED_ADAPTER_FEATURES) {
        bail!("GPU Adapter must support {:?}", REQUIRED_ADAPTER_FEATURES);
    }

    Ok(adapter)
}
