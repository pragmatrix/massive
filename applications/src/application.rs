use std::pin::Pin;

use anyhow::Result;
use derive_more::Debug;
use tokio::runtime::Builder;

use crate::{InstanceContext, McpId, ToolDeclaration};

/// An application that a host can run instances of.
///
/// The desktop runs many instances of each application it registers. The shell runs exactly one
/// instance of one application. Both hosts take the same value, so an application is written once
/// and runs under either.
///
/// A client builds it only through [`Application::new`] and the `with_` methods, and the host
/// checks the result. The desktop's registry checks the MCP id of every tool, for example.
#[derive(Debug)]
pub struct Application {
    name: String,
    runtime_kind: RuntimeKind,
    presentation: Presentation,
    tools: Option<ApplicationTools>,
    #[debug(skip)]
    run: RunInstanceBox,
}

/// The MCP tools an application declares, under its MCP id (ADR 0036).
#[derive(Debug)]
pub struct ApplicationTools {
    pub mcp_id: McpId,
    pub declarations: Vec<ToolDeclaration>,
}

/// The future of one running instance, as the host drives it.
pub type InstanceFuture = Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>;

type RunInstanceBox = Box<dyn Fn(InstanceContext) -> InstanceFuture + Send + Sync + 'static>;

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

/// How the instances of an application put their content on the screen.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Presentation {
    /// The instance creates views and submits instance changes, and its host draws them. Every
    /// host runs such an instance.
    Views,
    /// The instance opens its own windows and renders scene changes into them. Only the shell
    /// runs such an instance, and the desktop is one.
    Windows,
}

impl RuntimeKind {
    /// The builder of an instance runtime of this kind, which the host completes and passes to
    /// [`run_instance`](crate::run_instance).
    pub fn runtime_builder(self) -> Builder {
        match self {
            Self::CurrentThread => Builder::new_current_thread(),
            Self::MultiThread => Builder::new_multi_thread(),
        }
    }
}

impl Application {
    pub fn new<F, R>(name: impl Into<String>, run: F) -> Self
    where
        F: Fn(InstanceContext) -> R + Send + Sync + 'static,
        R: Future<Output = Result<()>> + Send + 'static,
    {
        let name = name.into();
        let run_boxed =
            Box::new(move |ctx: InstanceContext| -> InstanceFuture { Box::pin(run(ctx)) });

        Self {
            name,
            runtime_kind: RuntimeKind::CurrentThread,
            presentation: Presentation::Views,
            tools: None,
            run: run_boxed,
        }
    }

    /// Select the runtime flavor instances of this application run under (ADR 0009).
    /// Default is [`RuntimeKind::CurrentThread`].
    pub fn with_runtime_kind(mut self, runtime_kind: RuntimeKind) -> Self {
        self.runtime_kind = runtime_kind;
        self
    }

    /// Select how instances of this application present their content. Default is
    /// [`Presentation::Views`].
    pub fn with_presentation(mut self, presentation: Presentation) -> Self {
        self.presentation = presentation;
        self
    }

    /// Declares the MCP tools that agents may call on this application's instances (ADR 0036). The
    /// desktop joins `mcp_id` with each tool's own MCP id into the tool's name.
    pub fn with_mcp_id(mut self, mcp_id: McpId, declarations: Vec<ToolDeclaration>) -> Self {
        self.tools = Some(ApplicationTools {
            mcp_id,
            declarations,
        });
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn runtime_kind(&self) -> RuntimeKind {
        self.runtime_kind
    }

    pub fn presentation(&self) -> Presentation {
        self.presentation
    }

    pub fn tools(&self) -> Option<&ApplicationTools> {
        self.tools.as_ref()
    }

    /// Creates the future of a new instance. The host decides where and on which runtime it runs.
    pub fn run(&self, ctx: InstanceContext) -> InstanceFuture {
        (self.run)(ctx)
    }
}
