use std::sync::{Arc, Mutex};

use anyhow::Result;
use cosmic_text::{self as text, FontSystem};
use log::{error, info};
use wgpu::{Instance, InstanceDescriptor, PresentMode, Surface, SurfaceTarget, TextureFormat};
use winit::{
    dpi::PhysicalSize,
    event::*,
    event_loop::EventLoop,
    keyboard::{Key, NamedKey},
    window::{Window, WindowBuilder},
};

use massive_geometry::{scalar, Camera, Matrix4};
use massive_renderer::{Renderer, ShapeRenderer, ShapeRendererContext};
use massive_shapes::Shape;

pub trait Application {
    fn update(&mut self, window_event: WindowEvent);
    fn render(&self, shell: &mut Shell) -> (Camera, Vec<Shape>);
}

const Z_RANGE: (scalar, scalar) = (0.1, 100.0);

pub async fn run<A: Application + 'static>(
    application: A,
    font_system: Arc<Mutex<FontSystem>>,
) -> Result<()> {
    let event_loop = EventLoop::new()?;
    let window = WindowBuilder::new().build(&event_loop).unwrap();
    let mut shell = Shell::new(&window, window.inner_size(), font_system).await?;
    shell.run(event_loop, &window, application).await
}

pub struct Shell<'window> {
    pub font_system: Arc<Mutex<text::FontSystem>>,
    shape_renderer: ShapeRenderer,
    renderer: Renderer<'window>,
}

const DESIRED_MAXIMUM_FRAME_LATENCY: u32 = 1;

impl<'window> Shell<'window> {
    // Creating some of the wgpu types requires async code
    // TODO: We need the `FontSystem` only while rendering.
    pub async fn new(
        window: &Window,
        initial_size: PhysicalSize<u32>,
        font_system: Arc<Mutex<FontSystem>>,
    ) -> Result<Shell> {
        let instance_and_surface = Self::create_instance_and_surface(
            InstanceDescriptor::default(),
            // Use this for testing webgl:
            // InstanceDescriptor {
            //     backends: wgpu::Backends::GL,
            //     ..InstanceDescriptor::default()
            // },
            window,
        );
        // On wasm, attempt to fall back to webgl
        #[cfg(target_arch = "wasm32")]
        let instance_and_surface = match instance_and_surface {
            Ok(_) => instance_and_surface,
            Err(_) => Self::create_instance_and_surface(
                InstanceDescriptor {
                    backends: wgpu::Backends::GL,
                    ..InstanceDescriptor::default()
                },
                window,
            ),
        };
        let (instance, surface) = instance_and_surface?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::None,
                // Be sure the adapter can present the surface.
                compatible_surface: Some(&surface),
                // software fallback?
                force_fallback_adapter: false,
            })
            .await
            .expect("Adapter not found");

        info!("Effective WebGPU backend: {:?}", adapter.get_info().backend);

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::empty(),
                    // May be wrong, see: <https://github.com/gfx-rs/wgpu/blob/1144b065c4784d769d59da2f58f5aa13212627b0/examples/src/hello_triangle/mod.rs#L33-L34>
                    required_limits: adapter.limits(),
                    label: None,
                },
                None, // Trace path
            )
            .await
            .unwrap();

        let surface_caps = surface.get_capabilities(&adapter);

        // Don't use srgb now, colors are specified in linear rgb space.
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .unwrap_or(&surface_caps.formats[0]);

        info!("Surface format: {:?}", surface_format);

        let present_mode = surface_caps
            .present_modes
            .iter()
            .copied()
            .find(|f| *f == PresentMode::Immediate)
            .unwrap_or(surface_caps.present_modes[0]);

        info!(
            "Selecting present mode {:?}, initial size: {:?}",
            present_mode, initial_size
        );

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: *surface_format,
            width: initial_size.width,
            height: initial_size.height,
            present_mode,
            // TODO: Select this explicitly
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: DESIRED_MAXIMUM_FRAME_LATENCY,
        };
        surface.configure(&device, &surface_config);

        let renderer = Renderer::new(device, queue, surface, surface_config);

        Ok(Shell {
            font_system,
            shape_renderer: ShapeRenderer::default(),
            renderer,
        })
    }

    fn create_instance_and_surface(
        instance_descriptor: InstanceDescriptor,
        surface_target: &Window,
    ) -> Result<(Instance, Surface<'_>)> {
        let instance = wgpu::Instance::new(instance_descriptor);

        let surface_target: SurfaceTarget = surface_target.into();
        info!(
            "Creating surface on a {} target",
            match surface_target {
                SurfaceTarget::Window(_) => "Window",
                #[cfg(target_arch = "wasm32")]
                SurfaceTarget::Canvas(_) => "Canvas",
                #[cfg(target_arch = "wasm32")]
                SurfaceTarget::OffscreenCanvas(_) => "OffscreenCanvas",
                _ => "(Undefined SurfaceTarget, Internal Error)",
            }
        );

        let surface = instance.create_surface(surface_target)?;
        Ok((instance, surface))
    }

    pub async fn run<A: Application>(
        &mut self,
        event_loop: EventLoop<()>,
        window: &Window,
        mut application: A,
    ) -> Result<()> {
        info!("Entering event loop");
        event_loop.run(|event, window_target| {
            match event {
                Event::WindowEvent { event, window_id } if window_id == window.id() => {
                    info!("{:?}", event);
                    match event {
                        WindowEvent::CloseRequested
                        | WindowEvent::KeyboardInput {
                            event:
                                KeyEvent {
                                    state: ElementState::Pressed,
                                    logical_key: Key::Named(NamedKey::Escape),
                                    ..
                                },
                            ..
                        } => window_target.exit(),
                        WindowEvent::Resized(physical_size) => {
                            info!("{:?}", event);
                            self.resize_surface((physical_size.width, physical_size.height));
                            window.request_redraw()
                        }
                        WindowEvent::ScaleFactorChanged { .. } => {
                            let new_inner_size = window.inner_size();
                            self.resize_surface((new_inner_size.width, new_inner_size.height));
                            window.request_redraw()
                        }
                        WindowEvent::RedrawRequested => {
                            let (camera, shapes) = application.render(self);
                            let surface_matrix = self.renderer.surface_matrix();
                            let surface_size = self.renderer.surface_size();
                            let view_projection_matrix =
                                camera.view_projection_matrix(Z_RANGE, surface_size);

                            let primitives = {
                                // TODO: This is a mess.
                                let mut font_system = self.font_system.lock().unwrap();
                                let mut shape_renderer_context = ShapeRendererContext::new(
                                    &self.renderer.device,
                                    &self.renderer.queue,
                                    &self.renderer.texture_sampler,
                                    &self.renderer.texture_bind_group_layout,
                                    &mut font_system,
                                );
                                self.shape_renderer.render(
                                    &mut shape_renderer_context,
                                    &view_projection_matrix,
                                    &surface_matrix,
                                    &shapes,
                                )
                            };
                            // TODO: pass primitives as value.
                            match self
                                .renderer
                                .render_and_present(&view_projection_matrix, &primitives)
                            {
                                Ok(_) => {}
                                // Reconfigure the surface if lost
                                // TODO: shouldn't we redraw here? Also, I think the renderer can do this, too.
                                Err(wgpu::SurfaceError::Lost) => self.reconfigure_surface(),
                                // The system is out of memory, we should probably quit
                                Err(wgpu::SurfaceError::OutOfMemory) => window_target.exit(),
                                // All other errors (Outdated, Timeout) should be resolved by the next frame
                                Err(e) => error!("{:?}", e),
                            }
                        }

                        event => {
                            application.update(event);
                            window.request_redraw()
                        }
                    }
                }
                _ => {}
            }
        })?;
        Ok(())
    }

    /// The format chosen for the swapchain.
    pub fn surface_format(&self) -> TextureFormat {
        self.renderer.surface_config.format
    }

    /// A Matrix that translates from pixels (0,0)-(width,height) to screen space, which is -1.0 to
    /// 1.0 in each axis. Also flips y.
    pub fn pixel_matrix(&self) -> Matrix4 {
        let (_, surface_height) = self.renderer.surface_size();
        Matrix4::from_nonuniform_scale(1.0, -1.0, 1.0)
            * Matrix4::from_scale(1.0 / surface_height as f64 * 2.0)
    }

    fn resize_surface(&mut self, new_size: (u32, u32)) {
        self.renderer.resize_surface(new_size);
    }

    /// Reconfigure the surface after a change to the window's size or format.
    fn reconfigure_surface(&mut self) {
        self.renderer.reconfigure_surface()
    }

    // Surface size may not match the Window's size, for example if the window's size is 0,0.
    #[allow(unused)]
    fn surface_size(&self) -> (u32, u32) {
        self.renderer.surface_size()
    }
}

pub fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let r = f();
    println!("{name}: {:?}", start.elapsed());
    r
}
