//! The renderer's configuration

use std::{
    ops::Range,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use cosmic_text::FontSystem;
use derive_more::Debug;
use massive_shapes::Shape;

use crate::{
    renderer::{PreparationContext, RenderBatch},
    shape_renderer::{self, ShapeRenderer},
    text_layer::TextLayerRenderer,
};

// Naming: Somehow this is more than a config, batch produces may contain caches values.
#[derive(Debug)]
pub struct RendererConfig {
    pub measure: bool,
    pub batch_producers: Vec<BatchProducerInstance>,
}

impl RendererConfig {
    pub fn default_batch_producers(
        device: &wgpu::Device,
        font_system: Arc<Mutex<FontSystem>>,
        format: wgpu::TextureFormat,
    ) -> Vec<BatchProducerInstance> {
        let text_layer_renderer = TextLayerRenderer::new(device, font_system, format);
        let shape_renderer = ShapeRenderer::new::<shape_renderer::Vertex>(device, format);

        // Shapes are always rendered before (and therefore below) the text (for now).
        vec![
            BatchProducerInstance::new(Box::new(shape_renderer), 0..1),
            BatchProducerInstance::new(Box::new(text_layer_renderer), 1..3),
        ]
    }

    /// Returns all pipelines for all batch producers.
    pub fn create_pipelines(&self, device: &wgpu::Device) -> Vec<wgpu::RenderPipeline> {
        self.batch_producers
            .iter()
            .flat_map(|bp| bp.producer.create_pipelines(device))
            .collect()
    }
}

#[derive(Debug)]
pub struct BatchProducerInstance {
    #[debug(skip)]
    pub producer: Box<dyn BatchProducer>,
    pub pipeline_range: Range<usize>,
}

impl BatchProducerInstance {
    pub fn new(producer: Box<dyn BatchProducer>, pipeline_range: Range<usize>) -> Self {
        Self {
            producer,
            pipeline_range,
        }
    }
}

pub trait BatchProducer: Send {
    /// Create a new set of pipelines.
    ///
    /// This always has to be the same number of pipelines.
    fn create_pipelines(&self, device: &wgpu::Device) -> Vec<wgpu::RenderPipeline>;

    /// Produce batches for the pipelines.
    fn produce_batches(
        &mut self,
        context: &PreparationContext,
        shapes: &[Shape],
        batches: &mut [Option<RenderBatch>],
    ) -> Result<()>;
}

impl BatchProducer for TextLayerRenderer {
    fn create_pipelines(&self, device: &wgpu::Device) -> Vec<wgpu::RenderPipeline> {
        [
            self.create_sdf_pipeline(device),
            self.create_color_pipeline(device),
        ]
        .into()
    }

    /// We should require only &self here, everything that has cache semantics, should not require
    /// &mut self.
    fn produce_batches(
        &mut self,
        context: &PreparationContext,
        shapes: &[Shape],
        batches: &mut [Option<RenderBatch>],
    ) -> Result<()> {
        debug_assert_eq!(batches.len(), 2);

        let runs = shapes.iter().filter_map(|shape| {
            if let Shape::GlyphRun(run) = shape {
                Some(run)
            } else {
                None
            }
        });

        let mut produced_batches = self.runs_to_batches(context, runs)?;
        batches[0] = produced_batches[0].take();
        batches[1] = produced_batches[1].take();
        Ok(())
    }
}

impl BatchProducer for ShapeRenderer {
    fn create_pipelines(&self, device: &wgpu::Device) -> Vec<wgpu::RenderPipeline> {
        [self.create_pipeline(device)].into()
    }

    fn produce_batches(
        &mut self,
        context: &PreparationContext,
        shapes: &[Shape],
        batch_receiver: &mut [Option<RenderBatch>],
    ) -> Result<()> {
        debug_assert_eq!(batch_receiver.len(), 1);
        batch_receiver[0] = self.batch_from_shapes(context.device, shapes);
        Ok(())
    }
}
