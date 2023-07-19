use std::cell::RefCell;

use anyhow::Result;
use cosmic_text::{FontSystem, SwashCache};
use granularity::Value;
use log::{error, info};
use wgpu::{CommandBuffer, PresentMode, SurfaceTexture};
use winit::{
    event::*,
    event_loop::{ControlFlow, EventLoop},
    window::{Window, WindowBuilder},
};

#[derive(Clone)]
pub struct Shell {
    pub font_system: Value<RefCell<FontSystem>>,
    pub glyph_cache: Value<RefCell<SwashCache>>,
    pub surface: Value<wgpu::Surface>,
    pub device: Value<wgpu::Device>,
    pub queue: Value<wgpu::Queue>,
    pub surface_config: Value<wgpu::SurfaceConfiguration>,
}

impl Shell {
    // Creating some of the wgpu types requires async code
    async fn new(runtime: granularity::Runtime, window: &Window) -> Shell {
        let size = window.inner_size();

        // The instance is a handle to our GPU
        // Backends::all => Vulkan + Metal + DX12 + Browser WebGPU
        let instance = wgpu::Instance::default();

        // # Safety
        //
        // The surface needs to live as long as the window that created it.
        let surface = unsafe { instance.create_surface(&window) }.unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                // default: LowPower
                power_preference: wgpu::PowerPreference::LowPower,
                // Be sure the adapter can present the surface.
                compatible_surface: Some(&surface),
                // software fallback?
                force_fallback_adapter: false,
            })
            .await
            .expect("Adapter not found");

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    features: wgpu::Features::empty(),
                    // WebGL doesn't support all of wgpu's features, so if
                    // we're building for the web we'll have to disable some.
                    limits: if cfg!(target_arch = "wasm32") {
                        wgpu::Limits::downlevel_webgl2_defaults()
                    } else {
                        wgpu::Limits::default()
                    },
                    label: None,
                },
                None, // Trace path
            )
            .await
            .unwrap();

        let surface_caps = surface.get_capabilities(&adapter);

        // Shader code in this tutorial assumes an sRGB surface texture. Using a different
        // one will result all the colors coming out darker. If you want to support non
        // sRGB surfaces, you'll need to account for that when drawing to the frame.
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .unwrap_or(&surface_caps.formats[0]);

        let present_mode = surface_caps
            .present_modes
            .iter()
            .copied()
            .find(|f| *f == PresentMode::Immediate)
            .unwrap_or(surface_caps.present_modes[0]);

        info!("Selecting present mode {:?}", present_mode);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: *surface_format,
            width: size.width,
            height: size.height,
            present_mode,
            // TODO: Select this explicitly
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let font_system = runtime.var(FontSystem::new().into());
        let glyph_cache = runtime.var(SwashCache::new().into());
        // TODO: analyze dependencies and integrate the shell and its updates into the graph.
        let surface = runtime.var(surface);
        let device = runtime.var(device);
        let queue = runtime.var(queue);
        let config = runtime.var(config);

        Self {
            font_system,
            glyph_cache,
            surface,
            device,
            queue,
            surface_config: config,
        }
    }

    fn resize_surface(&mut self, new_size: (u32, u32)) {
        let new_surface_size = (new_size.0.max(1), new_size.1.max(1));

        if new_surface_size != self.surface_size() {
            self.surface_config.apply(|mut config| {
                config.width = new_surface_size.0;
                config.height = new_surface_size.1;
                config
            });

            self.reconfigure_surface();
        }
    }

    /// Reconfigure the surface after a change to the window's size or format.
    fn reconfigure_surface(&mut self) {
        self.surface.apply(|surface| {
            surface.configure(&self.device.get_ref(), &self.surface_config.get_ref());
            surface
        })
    }

    // Surface size may not match the Window's size, for example if the window's size is 0,0.
    fn surface_size(&self) -> (u32, u32) {
        let config = self.surface_config.get_ref();
        (config.width, config.height)
    }

    fn update(&mut self) {}

    fn render(
        &mut self,
        command_buffer: &mut Value<CommandBuffer>,
        surface_texture: &mut Value<SurfaceTexture>,
    ) -> Result<(), wgpu::SurfaceError> {
        self.queue.get_ref().submit([command_buffer.take()]);
        surface_texture.take().present();

        Ok(())
    }

    pub fn runtime(&self) -> granularity::Runtime {
        self.font_system.runtime()
    }
}

pub fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let r = f();
    println!("{name}: {:?}", start.elapsed());
    r
}

pub async fn run(
    runtime: granularity::Runtime,
    create_render_graph: impl FnOnce(&Shell) -> (Value<wgpu::CommandBuffer>, Value<wgpu::SurfaceTexture>)
        + 'static,
) -> Result<()> {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new().build(&event_loop).unwrap();
    let mut shell = Shell::new(runtime, &window).await;

    let (mut command_buffer, mut surface_texture) = create_render_graph(&shell);

    event_loop.run(move |event, _, control_flow| {
        match event {
            Event::WindowEvent {
                ref event,
                window_id,
            } if window_id == window.id() => {
                // UPDATED!
                match event {
                    WindowEvent::CloseRequested
                    | WindowEvent::KeyboardInput {
                        input:
                            KeyboardInput {
                                state: ElementState::Pressed,
                                virtual_keycode: Some(VirtualKeyCode::Escape),
                                ..
                            },
                        ..
                    } => *control_flow = ControlFlow::Exit,
                    WindowEvent::Resized(physical_size) => {
                        shell.resize_surface((physical_size.width, physical_size.height));
                        window.request_redraw()
                    }
                    WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                        shell.resize_surface((new_inner_size.width, new_inner_size.height));
                        window.request_redraw()
                    }
                    _ => {}
                }
            }

            Event::RedrawRequested(window_id) if window_id == window.id() => {
                shell.update();
                match shell.render(&mut command_buffer, &mut surface_texture) {
                    Ok(_) => {}
                    // Reconfigure the surface if lost
                    // TODO: shouldn't we redraw here?
                    Err(wgpu::SurfaceError::Lost) => shell.reconfigure_surface(),
                    // The system is out of memory, we should probably quit
                    Err(wgpu::SurfaceError::OutOfMemory) => *control_flow = ControlFlow::Exit,
                    // All other errors (Outdated, Timeout) should be resolved by the next frame
                    Err(e) => error!("{:?}", e),
                }
            }
            Event::MainEventsCleared => {
                // RedrawRequested will only trigger once, unless we manually
                // request it.
                // window.request_redraw();
            }
            _ => {}
        }
    });
}
