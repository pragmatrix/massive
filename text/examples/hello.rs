use granularity::Value;
use granularity_geometry::{Camera, Point3, Vector3};
use winit::event::{VirtualKeyCode, WindowEvent};

struct Application {
    camera: Value<Camera>,
    hello_world: Value<String>,
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let fovy = 45.0f64;
    let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();

    let camera = Camera::new(
        Point3::new(0.0, 0.0, camera_distance),
        Point3::new(0.0, 0.0, 0.0),
    );

    let runtime = granularity::Runtime::new();
    let camera = runtime.var(camera);

    let hello_world = runtime.var("Hello, world!".to_string());

    let application = Application {
        camera,
        hello_world,
    };

    // granularity_shell::run(application, move |application, shell| {
    //     granularity_text::render_graph(
    //         application.camera.clone(),
    //         application.hello_world.clone(),
    //         shell,
    //     )
    // })
    // .await;
}

impl Application {
    pub fn update(&mut self, window_event: WindowEvent<'static>) {
        if let WindowEvent::KeyboardInput { input, .. } = window_event {
            if input.state == winit::event::ElementState::Pressed {
                match input.virtual_keycode {
                    Some(VirtualKeyCode::Left) => self.camera.apply(|mut c| {
                        c.eye += Vector3::new(0.1, 0.0, 0.0);
                        c
                    }),
                    Some(VirtualKeyCode::Right) => self.camera.apply(|mut c| {
                        c.eye -= Vector3::new(0.1, 0.0, 0.0);
                        c
                    }),
                    Some(VirtualKeyCode::Up) => self.camera.apply(|mut c| {
                        c.eye += Vector3::new(0.0, 0.0, 0.1);
                        c
                    }),
                    Some(VirtualKeyCode::Down) => self.camera.apply(|mut c| {
                        c.eye -= Vector3::new(0.0, 0.0, 0.1);
                        c
                    }),
                    _ => {}
                }
            } else {
                {}
            }
        }
    }
}

// impl granularity_shell::Application for Application {
//     fn update(&mut self, window_event: WindowEvent<'static>) {
//         self.update(window_event)
//     }

//     fn runtime(&self) -> granularity::Runtime {
//         self.camera.runtime()
//     }
// }
