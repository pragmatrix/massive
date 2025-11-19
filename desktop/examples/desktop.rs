use anyhow::Result;
use desktop::{Application, Desktop};
use tokio::sync::mpsc;

use massive_applications::InstanceContext;
use massive_shell::{ApplicationContext, shell};

#[tokio::main]
async fn main() -> Result<()> {
    shell::run(run)
}

async fn run(ctx: ApplicationContext) -> Result<()> {
    let applications = vec![Application::new("Hello Application", hello_instance)];
    let desktop = Desktop::new(applications);
    desktop.run(ctx).await
}

async fn hello_instance(ctx: InstanceContext) -> Result<()> {
    println!(
        "Hello from the application instance {:?} with creation mode {:?}!",
        ctx.id(),
        ctx.creation_mode()
    );

    // Keep the application running indefinitely
    let (_tx, mut rx) = mpsc::channel::<()>(1);
    rx.recv().await;

    Ok(())
}
