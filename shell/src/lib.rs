use anyhow::Result;
use cosmic_text as text;
use cosmic_text::FontSystem;
use log::{error, info};
use wgpu::PresentMode;
use winit::{
    event::*,
    event_loop::{ControlFlow, EventLoop},
    window::{Window, WindowBuilder},
};

use granularity_geometry::{scalar, Camera, Matrix4};
use granularity_renderer::{Renderer, ShapeRenderer, ShapeRendererContext};
use granularity_shapes::Shape;

pub trait Application {
    fn update(&mut self, window_event: WindowEvent<'static>);
    fn render(&self, shell: &mut Shell) -> (Camera, Vec<Shape>);
}

const Z_RANGE: (scalar, scalar) = (0.1, 100.0);

pub async fn run<A: Application + 'static>(mut application: A) -> Result<()> {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new().build(&event_loop).unwrap();
    let mut shell = Shell::new(&window).await;

    event_loop.run(move |event, _, control_flow| {
        match event {
            Event::WindowEvent { event, window_id } if window_id == window.id() => match event {
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
                event => {
                    if let Some(static_event) = event.to_static() {
                        application.update(static_event);
                        window.request_redraw()
                    }
                }
            },
            Event::RedrawRequested(window_id) if window_id == window.id() => {
                let (camera, shapes) = application.render(&mut shell);
                let surface_matrix = shell.renderer.surface_matrix();
                let surface_size = shell.renderer.surface_size();
                // TODO: This is a mess.
                let mut shape_renderer_context = ShapeRendererContext {
                    device: &shell.renderer.device,
                    queue: &shell.renderer.queue,
                    texture_sampler: &shell.renderer.texture_sampler,
                    texture_bind_group_layout: &shell.renderer.texture_bind_group_layout,
                    font_system: &mut shell.font_system,
                };
                let view_projection_matrix = camera.view_projection_matrix(Z_RANGE, surface_size);
                let primitives = shell.shape_renderer.render(
                    &mut shape_renderer_context,
                    &view_projection_matrix,
                    &surface_matrix,
                    &shapes,
                );
                // TODO: pass primitives as value.
                match shell
                    .renderer
                    .render_and_present(&view_projection_matrix, &primitives)
                {
                    Ok(_) => {}
                    // Reconfigure the surface if lost
                    // TODO: shouldn't we redraw here? Also, I think the renderer can do this, too.
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

pub struct Shell {
    pub font_system: text::FontSystem,
    shape_renderer: ShapeRenderer,
    renderer: Renderer,
}

impl Shell {
    // Creating some of the wgpu types requires async code
    async fn new(window: &Window) -> Shell {
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
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: *surface_format,
            width: size.width,
            height: size.height,
            present_mode,
            // TODO: Select this explicitly
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &surface_config);

        let font_system = FontSystem::new();

        let renderer = Renderer::new(device, queue, surface, surface_config);

        Self {
            font_system,
            shape_renderer: ShapeRenderer::default(),
            renderer,
        }
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
