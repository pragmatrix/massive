use log::info;
use wgpu::PresentMode;
use winit::window::Window;

/// Create a new renderer that renders to a winit Window.
async fn new(window: &Window) -> Renderer {
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

    info!(
        "Selecting present mode {:?}, size: {:?}",
        present_mode, size
    );

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

    Renderer::new(device, queue, surface, config)
}
