use std::collections::HashMap;
use std::pin::Pin;

use anyhow::Result;
use derive_more::Debug;

use massive_applications::InstanceContext;

#[derive(Debug)]
pub struct ApplicationRegistry {
    applications: HashMap<String, Application>,
}

impl ApplicationRegistry {
    pub fn new(applications: Vec<Application>) -> Self {
        Self {
            applications: HashMap::from_iter(applications.into_iter().map(|a| (a.name.clone(), a))),
        }
    }

    pub fn get_named(&self, name: &str) -> Option<&Application> {
        self.applications.get(name)
    }
}

// Architecture: This probably belongs to massive_applications
#[derive(Debug)]
pub struct Application {
    pub(crate) name: String,
    pub(crate) runtime_kind: RuntimeKind,
    #[debug(skip)]
    pub(crate) run: RunInstanceBox,
}

type RunInstanceBox = Box<
    dyn Fn(InstanceContext) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>
        + Send
        + Sync
        + 'static,
>;

/// The tokio runtime flavor an instance runs under (ADR 0009).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RuntimeKind {
    /// Single-threaded default: no current instance has internal parallelism, so this
    /// keeps instance runtimes cheap.
    CurrentThread,
    /// Work-stealing worker pool for an application whose instance has internal
    /// parallelism (tokio::spawn, timers).
    MultiThread,
}

impl Application {
    pub fn new<F, R>(name: impl Into<String>, run: F) -> Self
    where
        F: Fn(InstanceContext) -> R + Send + Sync + 'static,
        R: Future<Output = Result<()>> + Send + 'static,
    {
        let name = name.into();
        let run_boxed = Box::new(
            move |ctx: InstanceContext| -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
                Box::pin(run(ctx))
            },
        );

        Self {
            name,
            runtime_kind: RuntimeKind::CurrentThread,
            run: run_boxed,
        }
    }

    /// Select the runtime flavor instances of this application run under (ADR 0009).
    /// Default is [`RuntimeKind::CurrentThread`].
    pub fn with_runtime_kind(mut self, runtime_kind: RuntimeKind) -> Self {
        self.runtime_kind = runtime_kind;
        self
    }
}
