//! A wrapper around a regular Scene that adds animation support.
use std::sync::Arc;

use anyhow::Result;
use derive_more::Deref;

use massive_animation::{AnimationContext, AnimationCoordinator};
use massive_renderer::{RenderPacing, RenderSubmission, RenderTarget};
use massive_scene::ChangeCollector;

#[derive(Debug, Deref)]
pub struct Scene {
    #[deref]
    inner: massive_scene::Scene,
    animation_coordinator: AnimationCoordinator,
}

impl AnimationContext for Scene {
    fn animation_coordinator(&self) -> &AnimationCoordinator {
        &self.animation_coordinator
    }
}

impl Scene {
    pub fn new(animation_coordinator: AnimationCoordinator) -> Self {
        Self::new_with_change_collector(animation_coordinator, Arc::new(ChangeCollector::default()))
    }

    pub fn new_with_change_collector(
        animation_coordinator: AnimationCoordinator,
        collector: Arc<ChangeCollector>,
    ) -> Self {
        // Robustness: We shouldn't allow arbitrary free generation of scenes anymore.
        let scene = massive_scene::Scene::new(collector);
        Self {
            inner: scene,
            animation_coordinator,
        }
    }


    pub(crate) fn from_parts(
        scene: massive_scene::Scene,
        animation_coordinator: AnimationCoordinator,
    ) -> Self {
        Self {
            inner: scene,
            animation_coordinator,
        }
    }

    // Render all the current scene changes.
    //
    // Pass in the current shell event if you need to handle redraw requests without scene changes
    // and automatic resizing of the renderer.
    pub fn render_to(&self, render_target: &mut dyn RenderTarget) -> Result<()> {
        render_target.render(self.begin_frame())
    }

    /// Take all changes from the Scene and return a RenderSubmission.
    pub fn begin_frame(&self) -> RenderSubmission {
        let animations_active = self.animation_coordinator.end_cycle();

        let pacing = if animations_active {
            RenderPacing::Smooth
        } else {
            RenderPacing::Fast
        };

        RenderSubmission::new(self.take_changes(), pacing)
    }
}
