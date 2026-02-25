//! The renderer's configuration

use std::ops::Range;

use anyhow::Result;
use derive_more::Debug;
use massive_geometry::Color;
use massive_shapes::Shape;

use crate::{
    FontManager,
    renderer::{PreparationContext, RenderBatch},
    shape_renderer::{self, ShapeRenderer},
    text_layer::TextLayerRenderer,
    tools::PipelineVariant,
};

pub const DEFAULT_BACKGROUND_COLOR: Color = Color::WHITE;

// Naming: Somehow this is more than a config, batch producers may contain caches.
#[derive(Debug)]
pub struct RendererConfig {
    pub surface_format: wgpu::TextureFormat,
    pub background_color: Option<Color>,
    pub measure: bool,
    pub batch_producers: Vec<BatchProducerInstance>,
}

impl RendererConfig {
    pub fn new(surface_format: wgpu::TextureFormat) -> Self {
        Self {
            surface_format,
            background_color: Some(DEFAULT_BACKGROUND_COLOR),
            batch_producers: Vec::new(),
            measure: false,
        }
    }

    pub fn with_default_batch_producers(
        device: &wgpu::Device,
        fonts: FontManager,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let mut config = Self::new(surface_format);
        config.add_batch_producer(
            ShapeRenderer::new::<shape_renderer::Vertex>(device, surface_format),
            1,
        );
        config.add_batch_producer(TextLayerRenderer::new(device, fonts, surface_format), 2);
        config
    }

    pub fn add_batch_producer(
        &mut self,
        batch_producer: impl BatchProducer + 'static,
        pipelines_count: usize,
    ) {
        let pipeline_start_index = self
            .batch_producers
            .last()
            .map(|i| i.pipeline_range.end)
            .unwrap_or(0usize);

        self.batch_producers.push(BatchProducerInstance {
            producer: Box::new(batch_producer),
            pipeline_range: pipeline_start_index..pipeline_start_index + pipelines_count,
        })
    }

    /// Creates all pipelines for all batch producers and one variant.
    pub fn create_pipelines(
        &self,
        device: &wgpu::Device,
        variant: PipelineVariant,
    ) -> Vec<wgpu::RenderPipeline> {
        self.batch_producers
            .iter()
            .flat_map(|bp| bp.producer.create_pipelines(device, variant))
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
    fn create_pipelines(
        &self,
        device: &wgpu::Device,
        variant: PipelineVariant,
    ) -> Vec<wgpu::RenderPipeline>;

    /// Produce batches for the pipelines.
    fn produce_batches(
        &mut self,
        context: &PreparationContext,
        shapes: &[Shape],
        batches: &mut [Option<RenderBatch>],
    ) -> Result<()>;
}

impl BatchProducer for TextLayerRenderer {
    fn create_pipelines(
        &self,
        device: &wgpu::Device,
        variant: PipelineVariant,
    ) -> Vec<wgpu::RenderPipeline> {
        [
            self.create_sdf_pipeline(device, variant),
            self.create_color_pipeline(device, variant),
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
    fn create_pipelines(
        &self,
        device: &wgpu::Device,
        variant: PipelineVariant,
    ) -> Vec<wgpu::RenderPipeline> {
        [self.create_pipeline(device, variant)].into()
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
