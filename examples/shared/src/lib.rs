pub mod application;
pub mod attributed_text;
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
        // Don't force initialization of the env logger (calling main may already initialized it)
        let _ = env_logger::try_init();

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
