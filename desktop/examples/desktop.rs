use anyhow::Result;
use desktop::{Application, Desktop};
use tokio::sync::mpsc;

use massive_applications::ApplicationContext;
use massive_shell::{ShellContext, shell};

#[tokio::main]
async fn main() -> Result<()> {
    shell::run(run)
}

async fn run(ctx: ShellContext) -> Result<()> {
    let applications = vec![Application::new("Hello Application", hello_app)];
    let desktop = Desktop::new(applications);
    desktop.run(ctx).await
}

async fn hello_app(_ctx: ApplicationContext) -> Result<()> {
    println!("Hello from the application!");

    // Keep the application running indefinitely
    let (_tx, mut rx) = mpsc::channel::<()>(1);
    rx.recv().await;

    Ok(())
}
