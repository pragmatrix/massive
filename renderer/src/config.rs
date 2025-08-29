//! The renderer's configuration

use anyhow::Result;
use derive_more::Debug;
use massive_shapes::Shape;

use crate::{
    renderer::{PreparationContext, RenderBatch},
    shape_renderer::ShapeRenderer,
    text_layer::TextLayerRenderer,
};

#[derive(Debug)]
pub struct RendererConfig {
    pub measure: bool,
    #[debug(skip)]
    pub batch_builders: Vec<Box<dyn BatchProducer>>,
}

impl RendererConfig {
    /// Returns a set of pipelines for each batch producer.
    pub fn create_pipeline_table(&self) -> Vec<Vec<wgpu::RenderPipeline>> {
        self.batch_builders
            .iter()
            .map(|bb| bb.pipelines())
            .collect()
    }
}

pub trait BatchProducer: Send {
    /// The pipelines used.
    fn pipelines(&self) -> Vec<wgpu::RenderPipeline>;

    /// Produce batches for the pipelines.
    fn produce_batches(
        &mut self,
        context: &PreparationContext,
        shapes: &[Shape],
        batches: &mut [Option<RenderBatch>],
    ) -> Result<()>;
}

impl BatchProducer for TextLayerRenderer {
    fn pipelines(&self) -> Vec<wgpu::RenderPipeline> {
        [self.sdf_pipeline().clone(), self.color_pipeline().clone()].into()
    }

    /// We should require only &self here, everything that has cache semantic, should not require
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
    fn pipelines(&self) -> Vec<wgpu::RenderPipeline> {
        [self.pipeline().clone()].into()
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
