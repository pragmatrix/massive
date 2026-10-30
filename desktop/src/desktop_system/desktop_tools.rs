//! The desktop as a tool provider (ADR 0036): binding a tool connection to the MCP root that covers
//! its target, and carrying out the desktop's tools. `docs/mcp-desktop-tools.md` specifies every
//! tool.

use std::sync::LazyLock;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use massive_applications::{
    AccessScope, InstanceId, McpId, ToolDeclaration, ToolError, ToolErrorCode, decode_arguments,
    encode_result, tool_name,
};
use massive_protocol::Target;
use massive_protocol::tools::v1::{ToolDescription, ToolOutcome};

use super::DesktopSystem;
use crate::instance_presenter::InstanceKind;
use crate::projects::{
    FullScreenMode, LaunchProfileId, LauncherMode, Params, ProjectId, SlotContent,
};
use crate::security::{self, CallingTool, ConnectingToTarget};

/// The MCP root a tool connection is bound to, and the tools its access scopes grant.
#[derive(Debug)]
pub struct ToolConnectionRoot {
    pub root: ProjectId,
    pub tools: Vec<ToolDescription>,
}

impl DesktopSystem {
    /// Binds a tool connection to the MCP root that covers `target`, or `None` to decline it.
    pub fn bind_tool_connection(&self, target: &Target) -> Option<ToolConnectionRoot> {
        let covering_mcp_root = self
            .target_project(target)
            .and_then(|project| self.aggregates.configuration.mcp_root_of(project));
        let root = match security::connecting_to_target(covering_mcp_root) {
            ConnectingToTarget::Bind(root) => root,
            ConnectingToTarget::Decline => return None,
        };
        let granted = self.granted_access_scopes(root);
        Some(ToolConnectionRoot {
            root,
            tools: DESKTOP_TOOLS
                .iter()
                .filter(|(_, declaration)| granted.contains(&declaration.access_scope))
                .map(|(_, declaration)| ToolDescription {
                    name: declaration.name.to_string(),
                    description: declaration.description.clone(),
                    input_schema: declaration.input_schema.as_value().to_string(),
                    output_schema: declaration.output_schema.as_value().to_string(),
                })
                .collect(),
        })
    }

    /// Carries out a call of the tool named `tool` on the MCP root `root`.
    pub fn call_tool(&self, root: ProjectId, tool: &str, arguments: &str) -> ToolOutcome {
        let outcome = self.try_call_tool(root, tool, arguments);
        match outcome {
            Ok(result) => ToolOutcome::Ok(result),
            Err(ToolError { code, message }) => ToolOutcome::Err {
                code: code.name().into(),
                message,
            },
        }
    }

    fn try_call_tool(
        &self,
        root: ProjectId,
        tool: &str,
        arguments: &str,
    ) -> Result<String, ToolError> {
        let Some((desktop_tool, declaration)) = DESKTOP_TOOLS
            .iter()
            .find(|(_, declaration)| declaration.name.is(tool))
        else {
            return Err(ToolError {
                code: ToolErrorCode::UnknownId,
                message: format!("No tool is named '{tool}'."),
            });
        };
        match security::calling_tool(self.granted_access_scopes(root), declaration.access_scope) {
            CallingTool::Allow => {}
            CallingTool::RefuseScopeNotGranted => {
                return Err(ToolError {
                    code: ToolErrorCode::ScopeNotGranted,
                    message: format!(
                        "The MCP project does not grant the access scope '{}'.",
                        declaration.access_scope.name()
                    ),
                });
            }
        }
        match desktop_tool {
            DesktopTool::Describe => {
                let DescribeParams {} = decode_arguments(arguments)?;
                Ok(encode_result(&self.describe_project(root)))
            }
        }
    }

    /// The project that holds `target`, or `None` when it names nothing.
    fn target_project(&self, target: &Target) -> Option<ProjectId> {
        let configuration = &self.aggregates.configuration;
        match target {
            Target::Instance(id) => {
                let instance: InstanceId = uuid::Uuid::parse_str(id).ok()?.into();
                self.aggregates.hierarchy.exists(&instance.into()).then(|| {
                    self.aggregates
                        .hierarchy
                        .project_of_target(&instance.into())
                })
            }
            Target::Path(path) => configuration
                .resolve_launcher_path(ProjectId::ROOT, path)
                .and_then(|launcher| configuration.project_of_launcher(launcher))
                .map(|project| project.id)
                .or_else(|| configuration.resolve_project_path(ProjectId::ROOT, path)),
        }
    }

    fn granted_access_scopes(&self, root: ProjectId) -> &massive_applications::AccessScopes {
        &self.aggregates.configuration[root]
            .mcp
            .as_ref()
            .expect("a bound tool connection's root is an MCP root")
            .access_scopes
    }

    fn describe_project(&self, project: ProjectId) -> ProjectDescription {
        let configuration = &self.aggregates.configuration;
        ProjectDescription {
            id: uuid::Uuid::from(project).to_string(),
            name: configuration[project].name.clone(),
            slots: configuration
                .slots_ordered(project)
                .map(|(position, content)| SlotDescription {
                    position: SlotPositionDescription {
                        column: position.column,
                        row: position.row,
                    },
                    launcher: match content {
                        SlotContent::Launcher(launcher) => Some(self.describe_launcher(launcher)),
                        SlotContent::Project(_) => None,
                    },
                    project: match content {
                        SlotContent::Project(nested) => Some(self.describe_project(nested)),
                        SlotContent::Launcher(_) => None,
                    },
                })
                .collect(),
        }
    }

    fn describe_launcher(&self, launcher: LaunchProfileId) -> LauncherDescription {
        let record = &self.aggregates.configuration[launcher];
        LauncherDescription {
            id: uuid::Uuid::from(launcher).to_string(),
            name: record.name.clone(),
            mode: match record.mode {
                LauncherMode::Visor => LauncherModeDescription::Visor,
                LauncherMode::Band => LauncherModeDescription::Band,
            },
            params: record.params.clone(),
            full_screen_mode: match record.full_screen_mode {
                FullScreenMode::Regular => FullScreenModeDescription::Regular,
                FullScreenMode::FullScreen => FullScreenModeDescription::Fullscreen,
            },
            instances: self
                .aggregates
                .hierarchy
                .launcher_instances(launcher)
                .map(|instance| self.describe_instance(instance))
                .collect(),
        }
    }

    fn describe_instance(&self, instance: InstanceId) -> InstanceDescription {
        let presenter = self
            .aggregates
            .instances
            .get(&instance)
            .expect("a running instance has a presenter");
        let title = presenter
            .primary_view_id()
            .and_then(|view| presenter.view_window_state(view).ok())
            .map(|state| state.title.clone())
            .unwrap_or_default();
        InstanceDescription {
            id: instance.to_string(),
            kind: match presenter.kind() {
                InstanceKind::Primary => InstanceKindDescription::Primary,
                InstanceKind::Assistant => InstanceKindDescription::Assistant,
            },
            title,
        }
    }
}

/// The desktop's tools, declared once, because their schemas never change.
static DESKTOP_TOOLS: LazyLock<Vec<(DesktopTool, ToolDeclaration)>> = LazyLock::new(|| {
    vec![(
        DesktopTool::Describe,
        ToolDeclaration::new::<DescribeParams, ProjectDescription>(
            tool_name!(McpId::DESKTOP, "describe"),
            AccessScope::Observe,
            "Returns the MCP project this connection is bound to and everything it covers: its \
             slots, nested projects, launchers, and running instances. Ids are valid only while \
             the desktop runs.",
        ),
    )]
});

#[derive(Debug, Clone, Copy)]
enum DesktopTool {
    Describe,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DescribeParams {}

#[derive(Debug, Serialize, JsonSchema)]
struct ProjectDescription {
    id: String,
    /// Names may repeat.
    name: String,
    /// Assigned slots only.
    slots: Vec<SlotDescription>,
}

/// Exactly one of `launcher` and `project` is set.
#[derive(Debug, Serialize, JsonSchema)]
struct SlotDescription {
    position: SlotPositionDescription,
    launcher: Option<LauncherDescription>,
    project: Option<ProjectDescription>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct SlotPositionDescription {
    /// Counted from 0.
    column: u32,
    /// Counted from 0.
    row: u32,
}

#[derive(Debug, Serialize, JsonSchema)]
struct LauncherDescription {
    id: String,
    name: String,
    mode: LauncherModeDescription,
    /// Passed to the application when an instance starts, such as a command to run.
    params: Params,
    full_screen_mode: FullScreenModeDescription,
    /// The launcher's running instances.
    instances: Vec<InstanceDescription>,
}

#[derive(Debug, Serialize, JsonSchema)]
struct InstanceDescription {
    id: String,
    kind: InstanceKindDescription,
    /// The title of the instance's primary view, empty while the application has set none.
    title: String,
}

/// How a launcher presents its instances.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum LauncherModeDescription {
    Visor,
    Band,
}

/// Whether a launcher's primary instances fill the window.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum FullScreenModeDescription {
    Regular,
    Fullscreen,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum InstanceKindDescription {
    Primary,
    Assistant,
}
