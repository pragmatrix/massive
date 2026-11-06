use std::collections::HashMap;

use anyhow::{Result, bail};

use massive_applications::{Application, ApplicationTools, DrawTarget, McpId};

#[derive(Debug)]
pub struct ApplicationRegistry {
    applications: HashMap<String, Application>,
}

impl ApplicationRegistry {
    /// Registers the applications the desktop can run.
    ///
    /// Fails when two applications share a name or an MCP id, when one uses the desktop's MCP id
    /// (ADR 0036), or when one draws its own windows, which only the shell can host. The
    /// applications come from the client's crate, such as mt's, so a mistake in them is the
    /// client's, and startup reports it as an error the client can show.
    pub fn new(applications: Vec<Application>) -> Result<Self> {
        let mut providers = HashMap::new();
        for application in &applications {
            if application.draw_target() == DrawTarget::Windows {
                bail!(
                    "the application '{}' draws its own windows, which only the shell can host",
                    application.name()
                );
            }
            let Some(tools) = application.tools() else {
                continue;
            };
            let name = application.name();
            if tools.mcp_id == McpId::DESKTOP {
                bail!("the application '{name}' uses the MCP id 'desktop', which is reserved");
            }
            if let Some(other) = providers.insert(tools.mcp_id, name) {
                bail!(
                    "the applications '{other}' and '{name}' both use the MCP id '{}'",
                    tools.mcp_id.as_str()
                );
            }
        }
        let mut registered = HashMap::new();
        for application in applications {
            if registered.contains_key(application.name()) {
                bail!("two applications are named '{}'", application.name());
            }
            registered.insert(application.name().to_owned(), application);
        }
        Ok(Self {
            applications: registered,
        })
    }

    pub fn get_named(&self, name: &str) -> Option<&Application> {
        self.applications.get(name)
    }

    /// The applications that declare MCP tools, by name.
    pub fn tool_providers(&self) -> impl Iterator<Item = (&str, &ApplicationTools)> {
        self.applications
            .values()
            .filter_map(|a| Some((a.name(), a.tools()?)))
    }
}

#[cfg(test)]
mod tests {
    use massive_applications::mcp_id;

    use super::*;

    fn application(name: &str, mcp_id: McpId) -> Application {
        Application::new(name, |_| async { Ok(()) }).with_tools(mcp_id, Vec::new())
    }

    #[test]
    fn the_registry_rejects_an_application_that_draws_its_own_windows() {
        let application =
            Application::new("a", |_| async { Ok(()) }).with_draw_target(DrawTarget::Windows);
        let error = ApplicationRegistry::new(vec![application]).unwrap_err();
        assert!(error.to_string().contains("only the shell"), "{error}");
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
        assert!(error.to_string().contains("both use the MCP id"), "{error}");
    }
}
