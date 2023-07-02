#[tokio::main]
async fn main() {
    env_logger::init();
    granularity_shell::run().await;
}
