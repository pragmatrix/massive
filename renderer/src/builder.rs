use massive_geometry::{Color, SizePx};

use crate::{
    FontManager, RenderDevice, Renderer, RendererConfig,
    shape_renderer::{self, ShapeRenderer},
    text_layer::TextLayerRenderer,
};

#[derive(Debug)]
pub struct RendererBuilder {
    pub device: RenderDevice,
    pub surface: wgpu::Surface<'static>,
    pub initial_size: SizePx,
    pub config: RendererConfig,
}

impl RendererBuilder {
    pub fn new(
        device: RenderDevice,
        surface: wgpu::Surface<'static>,
        initial_size: SizePx,
    ) -> Self {
        let surface_format = device.surface_format;
        Self {
            device,
            initial_size,
            surface,
            config: RendererConfig::new(surface_format),
        }
    }

    pub fn with_background_color(mut self, color: Color) -> Self {
        self.config.background_color = Some(color);
        self
    }

    pub fn with_shapes(mut self) -> Self {
        self.config.add_batch_producer(
            ShapeRenderer::new::<shape_renderer::Vertex>(
                &self.device.device,
                self.device.surface_format,
            ),
            1,
        );
        self
    }

    pub fn with_text(mut self, fonts: FontManager) -> Self {
        self.config.add_batch_producer(
            TextLayerRenderer::new(&self.device.device, fonts, self.device.surface_format),
            2,
        );
        self
    }

    pub fn with_measurements(mut self) -> Self {
        self.config.measure = true;
        self
    }

    pub fn build(self) -> Renderer {
        Renderer::new(self.device, self.surface, self.initial_size, self.config)
    }
}
