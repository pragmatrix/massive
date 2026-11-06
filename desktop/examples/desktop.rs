use anyhow::Result;
use tokio::sync::mpsc;

use massive_applications::{Application, InstanceContext};
use massive_desktop::{DesktopEnvironment, FontPolicy, ShapingEngineKind};

fn main() -> Result<()> {
    let applications = vec![Application::new("Hello Application", hello_instance)];
    DesktopEnvironment::new(applications)?.run(FontPolicy::bare(ShapingEngineKind::CosmicText))
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
