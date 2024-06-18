use std::{
    future::Future,
    mem,
    sync::{Arc, Mutex},
    task::{self, Waker},
};

use anyhow::Result;
use cosmic_text::{self as text, FontSystem};
use futures::{task::ArcWake, FutureExt};
use log::{error, info};
use massive_scene::{Director, SceneChange};
use tokio::{
    sync::{
        mpsc::{self, channel, Sender},
        oneshot,
    },
    task::LocalSet,
};
use wgpu::{Instance, InstanceDescriptor, PresentMode, Surface, SurfaceTarget, TextureFormat};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{EventLoop, EventLoopProxy},
    window::Window,
};

use massive_geometry::{scalar, Camera, Matrix4};
use massive_renderer::Renderer;

const Z_RANGE: (scalar, scalar) = (0.1, 100.0);

pub struct Shell2<'window> {
    pub font_system: Arc<Mutex<text::FontSystem>>,
    renderer: Renderer<'window>,
    initial_size: PhysicalSize<u32>,
}

const DESIRED_MAXIMUM_FRAME_LATENCY: u32 = 1;

impl<'window> Shell2<'window> {
    pub fn event_loop() -> Result<EventLoop<ShellEvent>> {
        Ok(EventLoop::with_user_event().build()?)
    }

    // Creating some of the wgpu types requires async code
    // TODO: We need the `FontSystem` only while rendering.
    pub async fn new(
        window: &Window,
        initial_size: PhysicalSize<u32>,
        font_system: Arc<Mutex<FontSystem>>,
    ) -> Result<Shell2> {
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

        let alpha_mode = surface_caps.alpha_modes[0];

        info!(
            "Selecting present mode {:?}, alpha mode: {:?}, initial size: {:?}",
            present_mode, alpha_mode, initial_size
        );

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: *surface_format,
            width: initial_size.width,
            height: initial_size.height,
            present_mode,
            // TODO: Select this explicitly
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: DESIRED_MAXIMUM_FRAME_LATENCY,
        };
        surface.configure(&device, &surface_config);

        let renderer = Renderer::new(device, queue, surface, surface_config);

        Ok(Shell2 {
            font_system,
            renderer,
            initial_size,
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

    pub async fn run<R: Future<Output = Result<()>> + 'static>(
        &mut self,
        event_loop: EventLoop<ShellEvent>,
        window: &'window Window,
        // TODO: Move Camera into the application
        camera: Camera,
        application: impl FnOnce(ApplicationContext) -> R + 'static,
    ) -> Result<()> {
        // Spawn application.

        // TODO: may use unbounded channels.
        let (scene_sender, mut scene_receiver) = channel::<Vec<SceneChange>>(256);
        let (event_sender, event_receiver) = channel(256);

        let proxy = event_loop.create_proxy();
        let scene_proxy = proxy.clone();

        let event_dispatcher_task = tokio::spawn(async move {
            loop {
                if let Some(new_scene_changes) = scene_receiver.recv().await {
                    scene_proxy.send_event(ShellEvent::SceneChanges(new_scene_changes))?;
                } else {
                    info!("Scene change -> Event dispatcher ended, no more senders.");
                    return Result::<()>::Ok(());
                }
            }
        });

        let application_context = ApplicationContext {
            upload_channel: Some(scene_sender),
            window_events: event_receiver,
            initial_window_size: self.initial_size,
            window_scale_factor: window.scale_factor(),
            font_system: self.font_system.clone(),
            camera,
        };

        let local_set = LocalSet::new();
        let (result_tx, mut result_rx) = oneshot::channel();
        let _application_task = local_set.spawn_local(async move {
            let r = application(application_context).await;
            // Found no way to retrieve the result via JoinHandle, so this must do.
            result_tx
                .send(Some(r))
                .expect("Internal Error: Failed to set the application result");
        });

        // Event loop

        {
            // Shared state to wake the event loop
            let waker = Arc::new(EventLoopWaker {
                proxy: proxy.clone(),
                waker: Mutex::new(None),
            });

            let mut winit_context = WinitApplicationHandler::<'_, 'window> {
                shell: self,
                window,
                camera,
                event_sender,
                scene_changes: Default::default(),
                local_set,
                waker,
            };

            info!("Entering event loop");
            event_loop.run_app(&mut winit_context)?;
            info!("Exiting event loop");
        }

        // LocalSet is dropped. Event dispatcher should end now without an error.
        event_dispatcher_task.await??;

        // Check application's result
        if let Ok(Some(r)) = result_rx.try_recv() {
            info!("Application ended with {:?}", r);
            r?;
        } else {
            // TODO: This should probably be an error, we want to the application to have full
            // control over the lifetime of the winit event loop.
            info!("Application did not end");
        }

        Ok(())
    }

    /// The format chosen for the swapchain.
    pub fn surface_format(&self) -> TextureFormat {
        self.renderer.surface_config.format
    }

    /// A Matrix that translates from pixels (0,0)-(width,height) to screen space, which is -1.0 to
    /// 1.0 in each axis. Also flips y.
    pub fn pixel_matrix(&self) -> Matrix4 {
        self.renderer.pixel_matrix()
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

#[allow(unused)]
pub fn time<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let r = f();
    println!("{name}: {:?}", start.elapsed());
    r
}

// Rationale: We can't pre-create a `Director`, because it contains `Rc`, which is not send.
#[derive(Debug)]
pub struct ApplicationContext {
    upload_channel: Option<Sender<Vec<SceneChange>>>,
    pub window_events: mpsc::Receiver<WindowEvent>,
    pub initial_window_size: PhysicalSize<u32>,
    pub window_scale_factor: f64,
    pub font_system: Arc<Mutex<FontSystem>>,
    pub camera: Camera,
}

impl ApplicationContext {
    pub fn director(&mut self) -> Director {
        Director::from_sender(
            self.upload_channel
                .take()
                .expect("Only one director can be created"),
        )
    }
}

struct WinitApplicationHandler<'shell, 'window> {
    shell: &'shell mut Shell2<'window>,
    window: &'window Window,
    camera: Camera,
    event_sender: Sender<WindowEvent>,
    scene_changes: Vec<SceneChange>,
    local_set: LocalSet,
    waker: Arc<EventLoopWaker>,
}

impl ApplicationHandler<ShellEvent> for WinitApplicationHandler<'_, '_> {
    fn resumed(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        // TODO: create the window here.
    }

    fn user_event(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, event: ShellEvent) {
        match event {
            ShellEvent::WakeUpApplication => {
                self.drive_application(event_loop);
            }
            ShellEvent::SceneChanges(new_changes) => {
                self.scene_changes.extend(new_changes);
                self.window.request_redraw();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        info!("{:?}", event);
        match event {
            WindowEvent::Resized(physical_size) => {
                info!("{:?}", event);
                self.shell
                    .resize_surface((physical_size.width, physical_size.height));
                self.window.request_redraw()
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let new_inner_size = self.window.inner_size();
                self.shell
                    .resize_surface((new_inner_size.width, new_inner_size.height));
                self.window.request_redraw()
            }
            WindowEvent::RedrawRequested => {
                self.redraw(event_loop);
            }

            event => {
                if let Err(_e) = self.event_sender.try_send(event) {
                    info!("Receiver for events dropped, exiting");
                    event_loop.exit();
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.drive_application(event_loop);
    }
}

impl WinitApplicationHandler<'_, '_> {
    fn redraw(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let new_changes: Vec<_> = mem::take(&mut self.scene_changes);

        // let surface_matrix = self.renderer.surface_matrix();
        let surface_size = self.shell.renderer.surface_size();
        let view_projection_matrix = self.camera.view_projection_matrix(Z_RANGE, surface_size);

        // let surface_view_matrix = surface_matrix * view_projection_matrix;

        {
            let mut font_system = self.shell.font_system.lock().unwrap();
            self.shell
                .renderer
                .apply_changes(&mut font_system, new_changes)
                .expect("Render preparations failed");
        }

        // TODO: pass primitives as value.
        match self
            .shell
            .renderer
            .render_and_present(&view_projection_matrix)
        {
            Ok(_) => {}
            // Reconfigure the surface if lost
            // TODO: shouldn't we redraw here? Also, I think the renderer can do this, too.
            Err(wgpu::SurfaceError::Lost) => self.shell.reconfigure_surface(),
            // The system is out of memory, we should probably quit
            Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
            // All other errors (Outdated, Timeout) should be resolved by the next frame
            Err(e) => error!("{:?}", e),
        }
    }

    fn drive_application(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let waker_ref = futures::task::waker_ref(&self.waker);
        let mut context = task::Context::from_waker(&waker_ref);
        if self.local_set.poll_unpin(&mut context).is_ready() {
            event_loop.exit();
        }
    }
}

#[derive(Debug)]
pub enum ShellEvent {
    WakeUpApplication,
    SceneChanges(Vec<SceneChange>),
}

struct EventLoopWaker {
    proxy: EventLoopProxy<ShellEvent>,
    waker: Mutex<Option<Waker>>,
}

impl ArcWake for EventLoopWaker {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        if let Ok(mut waker_guard) = arc_self.waker.lock() {
            if let Some(waker) = waker_guard.take() {
                waker.wake();
            }
        }
        let _ = arc_self.proxy.send_event(ShellEvent::WakeUpApplication);
    }
}
