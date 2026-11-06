//! Reproduces a lost instance result for an instance that draws into its own windows.
//!
//! Such an instance owns its event receiver inside its context, so the receiver closes when the
//! context drops, before the instance's task reports the end. A window event in that gap makes the
//! shell's send fail, and the shell ends its event loop without the instance's result. The desktop
//! has the same order, with a gap of microseconds. This example widens the gap to half a second and
//! fills it with a redraw request.
//!
//! Expected: `shell::run` returns the instance's own error. Reproduced: it returns "Event loop
//! exited, but application did not end".

use std::time::Duration;

use anyhow::{Result, anyhow};

use massive_applications::{Application, DrawTarget, InstanceContext};
use massive_renderer::{FontPolicy, ShapingEngineKind};
use massive_shell::{InstanceWindows, shell};

const INSTANCE_ERROR: &str = "the instance's own error";

fn main() {
    let application =
        Application::new("lost result", instance).with_draw_target(DrawTarget::Windows);
    let result = shell::run(
        application,
        FontPolicy::bare(ShapingEngineKind::available()[0]),
    );
    match result {
        Err(error) if error.to_string() == INSTANCE_ERROR => {
            println!("not reproduced: the shell returned the instance's result");
        }
        other => {
            println!("reproduced: the shell returned {other:?} instead of the instance's result");
            std::process::exit(1);
        }
    }
}

async fn instance(context: InstanceContext) -> Result<()> {
    let window = context.new_window((400, 300)).await?;
    // The receiver closes here, as it does when the desktop's context drops at its end.
    drop(context);
    // A window event in the gap, which the shell tries to send to the closed receiver.
    window.request_redraw();
    tokio::time::sleep(Duration::from_millis(500)).await;
    Err(anyhow!(INSTANCE_ERROR))
}
