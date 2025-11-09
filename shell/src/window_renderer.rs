use std::sync::Arc;

use anyhow::{Context, Result};
use wgpu::{PresentMode, TextureFormat};
use winit::window::WindowId;

use crate::shell_window::ShellWindowShared;
use massive_geometry::{Color, Matrix4};
use massive_renderer::Renderer;
use massive_scene::ChangeCollector;

pub struct WindowRenderer {
    window: Arc<ShellWindowShared>,
    renderer: Renderer,
    change_collector: Arc<ChangeCollector>,
}

impl WindowRenderer {
    pub fn new(window: Arc<ShellWindowShared>, renderer: Renderer) -> Self {
        Self {
            window,
            renderer,
            change_collector: ChangeCollector::default().into(),
        }
    }

    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub fn change_collector(&self) -> &Arc<ChangeCollector> {
        &self.change_collector
    }

    /// The format chosen for the swapchain.
    pub fn surface_format(&self) -> TextureFormat {
        self.renderer.surface_config.format
    }

    // Surface size may not match the Window's size, for example if the window's size is 0,0.
    pub fn surface_size(&self) -> (u32, u32) {
        self.renderer.surface_size()
    }

    /// Sets the background color for the next redraw.
    ///
    /// Does not request a redraw.
    pub fn set_background_color(&mut self, color: Option<Color>) {
        self.renderer.set_background_color(color);
    }
}

impl WindowRenderer {
    pub(crate) fn resize(&mut self, new_size: (u32, u32)) {
        self.renderer.resize_surface(new_size)
    }

    pub(crate) fn set_present_mode(&mut self, present_mode: PresentMode) {
        self.renderer.set_present_mode(present_mode);
    }

    /// Apply all changes to the renderer and prepare the presentation.
    ///
    /// This is separate from render_and_present.
    ///
    /// Detail: This blocks in VSync modes until the previous frame is presented.
    pub(crate) fn apply_scene_changes_and_prepare_presentation(
        &mut self,
    ) -> Result<wgpu::SurfaceTexture> {
        let changes = self.change_collector.take_all();

        self.renderer.apply_changes(changes)?;

        self.renderer.prepare()?;

        // Robustness: Learn about how to recover from specific `SurfaceError`s errors here
        // `get_current_texture()` tries once.
        let texture = self
            .renderer
            .get_current_texture()
            .context("get_current_texture")?;

        Ok(texture)
    }

    pub(crate) fn render_and_present(
        &mut self,
        view_projection_matrix: Matrix4,
        texture: wgpu::SurfaceTexture,
    ) {
        self.renderer
            .render_and_present(&view_projection_matrix, texture)
    }
}
