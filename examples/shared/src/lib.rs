pub mod application2;
pub mod code_viewer;
pub mod fonts;
pub mod positioning;

use std::future::Future;

use anyhow::Result;

pub fn main<Fut>(main: impl FnOnce() -> Fut + 'static) -> Result<()>
where
    Fut: Future<Output = Result<()>>,
{
    #[cfg(not(target_arch = "wasm32"))]
    {
        env_logger::init();

        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        // Use the runtime to block on the async function
        rt.block_on(main())
    }

    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        console_log::init().expect("Could not initialize logger");

        wasm_bindgen_futures::spawn_local(async {
            match main().await {
                Ok(()) => {}
                Err(e) => {
                    log::error!("{e}");
                }
            }
        });

        Ok(())
    }
}
