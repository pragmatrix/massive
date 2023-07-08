use granularity_geometry::{Camera, Point3};

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

    granularity_shell::run(runtime, move |shell| {
        granularity_text::render_graph(camera, shell)
    })
    .await;
}
