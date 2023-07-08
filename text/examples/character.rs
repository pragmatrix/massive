#[tokio::main]
async fn main() {
    env_logger::init();

    granularity_shell::run(granularity_text::render_graph).await;
}
