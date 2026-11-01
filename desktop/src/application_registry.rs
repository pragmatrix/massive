use std::collections::{HashMap, HashSet};
use std::pin::Pin;

use anyhow::Result;
use derive_more::Debug;

use massive_applications::{InstanceContext, McpId, ToolDeclaration};

#[derive(Debug)]
pub struct ApplicationRegistry {
    applications: HashMap<String, Application>,
}

impl ApplicationRegistry {
    /// Panics when an application uses the desktop's MCP id or one another application uses,
    /// because applications and their MCP ids come from code (ADR 0036).
    pub fn new(applications: Vec<Application>) -> Self {
        let mut mcp_ids = HashSet::new();
        for tools in applications.iter().filter_map(|a| a.tools.as_ref()) {
            assert_ne!(
                tools.mcp_id,
                McpId::DESKTOP,
                "the MCP id 'desktop' is reserved for the desktop's tools"
            );
            assert!(
                mcp_ids.insert(tools.mcp_id),
                "two applications use the MCP id '{}'",
                tools.mcp_id.as_str()
            );
        }
        Self {
            applications: HashMap::from_iter(applications.into_iter().map(|a| (a.name.clone(), a))),
        }
    }

    pub fn get_named(&self, name: &str) -> Option<&Application> {
        self.applications.get(name)
    }

    /// The applications that declare MCP tools, by name.
    pub fn tool_providers(&self) -> impl Iterator<Item = (&str, &ApplicationTools)> {
        self.applications
            .values()
            .filter_map(|a| Some((a.name.as_str(), a.tools.as_ref()?)))
    }
}

// Architecture: This probably belongs to massive_applications
/// An application the desktop can run instances of.
///
/// The crate exports it, so its fields stay `pub(crate)`. A client builds it only through
/// [`Application::new`] and the `with_` methods, which check what they set, such as the MCP id of
/// every tool.
#[derive(Debug)]
pub struct Application {
    pub(crate) name: String,
    pub(crate) runtime_kind: RuntimeKind,
    pub(crate) tools: Option<ApplicationTools>,
    #[debug(skip)]
    pub(crate) run: RunInstanceBox,
}

/// The MCP tools an application declares, under its MCP id (ADR 0036).
#[derive(Debug)]
pub struct ApplicationTools {
    pub mcp_id: McpId,
    pub declarations: Vec<ToolDeclaration>,
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

    /// Declares the MCP tools that agents may call on this application's instances (ADR 0036).
    /// Panics when a tool's name belongs to another MCP id.
    pub fn with_mcp_id(mut self, mcp_id: McpId, declarations: Vec<ToolDeclaration>) -> Self {
        for declaration in &declarations {
            assert_eq!(
                declaration.name.provider(),
                mcp_id,
                "the tool '{}' belongs to another MCP id",
                declaration.name
            );
        }
        self.tools = Some(ApplicationTools {
            mcp_id,
            declarations,
        });
        self
    }
}

#[cfg(test)]
mod tests {
    use massive_applications::mcp_id;

    use super::*;

    fn application(name: &str, mcp_id: McpId) -> Application {
        Application::new(name, |_| async { Ok(()) }).with_mcp_id(mcp_id, Vec::new())
    }

    #[test]
    #[should_panic(expected = "reserved")]
    fn the_registry_rejects_the_desktops_mcp_id() {
        ApplicationRegistry::new(vec![application("a", McpId::DESKTOP)]);
    }

    #[test]
    #[should_panic(expected = "two applications")]
    fn the_registry_rejects_a_duplicate_mcp_id() {
        let id = mcp_id!("term");
        ApplicationRegistry::new(vec![application("a", id), application("b", id)]);
    }
}
