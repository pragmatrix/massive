//! Every tool an agent can call, the desktop's and the applications', declared once when the
//! desktop starts (ADR 0036).

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use massive_applications::{AccessScope, AccessScopes, ToolDeclaration, ToolName};
use massive_protocol::tools::v1::ToolDescription;

use super::desktop_tools;
use crate::application_registry::ApplicationRegistry;

/// The tools in the order a tool connection lists them, the desktop's first.
#[derive(Debug)]
pub struct ToolCatalog {
    tools: Vec<(ToolDescription, AccessScope)>,
    application_tools: HashMap<String, ApplicationTool>,
}

/// A tool an application declares. The desktop forwards its calls to the instance that the
/// argument `instance_id` names.
#[derive(Debug)]
pub struct ApplicationTool {
    pub application: String,
    pub name: ToolName,
    pub access_scope: AccessScope,
}

impl ToolCatalog {
    /// Collects the desktop's tools and the tools of every registered application.
    ///
    /// Panics when two tools have the same name. A joined name can collide even though MCP ids
    /// differ, such as the application `desktop_assign`'s tool `slot` with `desktop_assign_slot`.
    pub fn new(applications: &ApplicationRegistry) -> Self {
        let mut tools: Vec<_> = desktop_tools::declarations()
            .map(|declaration| {
                (
                    description(declaration, declaration.input_schema.as_value().clone()),
                    declaration.access_scope,
                )
            })
            .collect();
        let mut application_tools = HashMap::new();
        for (application, provider) in applications.tool_providers() {
            for declaration in &provider.declarations {
                tools.push((
                    description(declaration, with_instance_id(declaration)),
                    declaration.access_scope,
                ));
                application_tools.insert(
                    declaration.name.to_string(),
                    ApplicationTool {
                        application: application.into(),
                        name: declaration.name,
                        access_scope: declaration.access_scope,
                    },
                );
            }
        }
        let mut names = HashSet::new();
        for (tool, _) in &tools {
            assert!(
                names.insert(&tool.name),
                "two tools are named '{}'",
                tool.name
            );
        }
        Self {
            tools,
            application_tools,
        }
    }

    /// The tools `granted` grants, as a tool connection lists them.
    pub fn granted(&self, granted: &AccessScopes) -> Vec<ToolDescription> {
        self.tools
            .iter()
            .filter(|(_, access_scope)| granted.contains(access_scope))
            .map(|(tool, _)| tool.clone())
            .collect()
    }

    pub fn application_tool(&self, name: &str) -> Option<&ApplicationTool> {
        self.application_tools.get(name)
    }
}

fn description(declaration: &ToolDeclaration, input_schema: Value) -> ToolDescription {
    ToolDescription {
        name: declaration.name.to_string(),
        description: declaration.description.clone(),
        input_schema: input_schema.to_string(),
        output_schema: declaration.output_schema.as_value().to_string(),
    }
}

/// The input schema of an application's tool, with the `instance_id` that routes its calls. The
/// application never sees that argument, so its parameter type does not declare it.
fn with_instance_id(declaration: &ToolDeclaration) -> Value {
    let mut schema = declaration.input_schema.as_value().clone();
    let object = schema
        .as_object_mut()
        .filter(|object| object.get("type") == Some(&json!("object")))
        .unwrap_or_else(|| panic!("the params of the tool '{}' are a struct", declaration.name));
    object
        .entry("properties")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("a schema's properties are an object")
        .insert(
            "instance_id".into(),
            json!({
                "type": "string",
                "description": "The id of the instance to call, as desktop_describe lists it."
            }),
        );
    object
        .entry("required")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .expect("a schema's required properties are an array")
        .push(json!("instance_id"));
    schema
}

#[cfg(test)]
mod tests {
    use massive_applications::{AccessScope, McpId, ToolDeclaration, mcp_id, tool_name};

    use super::*;
    use crate::Application;

    #[derive(schemars::JsonSchema)]
    struct NoParams {}

    #[test]
    #[should_panic(expected = "two tools are named 'desktop_assign_slot'")]
    fn a_joined_name_that_collides_with_another_tool_panics() {
        const PROVIDER: McpId = mcp_id!("desktop_assign");
        let colliding = ToolDeclaration::new::<NoParams, NoParams>(
            tool_name!(PROVIDER, "slot"),
            AccessScope::Read,
            "",
        );
        let application =
            Application::new("a", |_| async { Ok(()) }).with_mcp_id(PROVIDER, vec![colliding]);
        ToolCatalog::new(&ApplicationRegistry::new(vec![application]));
    }
}
