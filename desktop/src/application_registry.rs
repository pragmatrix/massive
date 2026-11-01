use std::collections::{HashMap, HashSet};
use std::pin::Pin;

use anyhow::{Result, bail};
use derive_more::Debug;

use massive_applications::{InstanceContext, McpId, ToolDeclaration};

#[derive(Debug)]
pub struct ApplicationRegistry {
    applications: HashMap<String, Application>,
}

impl ApplicationRegistry {
    /// Registers the applications the desktop can run.
    ///
    /// Fails when an application uses the desktop's MCP id or one another application uses, or
    /// declares a tool whose name belongs to another MCP id (ADR 0036).
    pub fn new(applications: Vec<Application>) -> Result<Self> {
        let mut mcp_ids = HashSet::new();
        for tools in applications.iter().filter_map(|a| a.tools.as_ref()) {
            if tools.mcp_id == McpId::DESKTOP {
                bail!("the MCP id 'desktop' is reserved for the desktop's tools");
            }
            if !mcp_ids.insert(tools.mcp_id) {
                bail!(
                    "two applications use the MCP id '{}'",
                    tools.mcp_id.as_str()
                );
            }
            if let Some(declaration) = tools
                .declarations
                .iter()
                .find(|declaration| declaration.name.provider() != tools.mcp_id)
            {
                bail!(
                    "the tool '{}' does not belong to the MCP id '{}'",
                    declaration.name,
                    tools.mcp_id.as_str()
                );
            }
        }
        Ok(Self {
            applications: HashMap::from_iter(applications.into_iter().map(|a| (a.name.clone(), a))),
        })
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
/// [`Application::new`] and the `with_` methods, and the [`ApplicationRegistry`] checks the result,
/// such as the MCP id of every tool.
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
    pub fn with_mcp_id(mut self, mcp_id: McpId, declarations: Vec<ToolDeclaration>) -> Self {
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
    fn the_registry_rejects_the_desktops_mcp_id() {
        let error = ApplicationRegistry::new(vec![application("a", McpId::DESKTOP)]).unwrap_err();
        assert!(error.to_string().contains("reserved"), "{error}");
    }

    #[test]
    fn the_registry_rejects_a_duplicate_mcp_id() {
        let id = mcp_id!("term");
        let error =
            ApplicationRegistry::new(vec![application("a", id), application("b", id)]).unwrap_err();
        assert!(error.to_string().contains("two applications"), "{error}");
    }
}
