//! The desktop as a tool provider (ADR 0036): binding a tool connection to the MCP root that covers
//! its target, and carrying out the desktop's tools. `docs/mcp-desktop-tools.md` specifies every
//! tool.

use std::borrow::Cow;
use std::sync::LazyLock;

use derive_more::Into;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use massive_applications::{
    AccessScope, AccessScopes, InstanceId, McpId, ToolDeclaration, ToolError, ToolErrorCode,
    ToolResultJson, decode_arguments, encode_result, tool_name,
};
use massive_protocol::Target;
use massive_protocol::tools::v1::{ToolDescription, ToolOutcome};

use super::{
    ConfigurationCommand, DesktopCommand, DesktopSystem, Direction, FocusIntent, SlotShift,
};
use crate::instance_presenter::InstanceKind;
use crate::policy::constants;
use crate::policy::when::{self, NestingParams};
use crate::projects::{
    FullScreenMode, LaunchProfile, LaunchProfileId, LauncherMode, Params, ProjectId,
    SlotAssignment, SlotContent, SlotPosition, params_depth,
};
use crate::security::{
    self, CallingTool, ConnectingToTarget, CreatingLauncher, NamingSlot, NamingTarget,
};

/// The MCP root a tool connection is bound to, and the tools its access scopes grant.
#[derive(Debug)]
pub struct ToolConnectionRoot {
    pub root: ProjectId,
    pub tools: Vec<ToolDescription>,
}

/// How the desktop carries out a tool call: answered from its state, or by a command that the
/// desktop loop runs before it answers.
#[derive(Debug)]
pub enum ToolCallPlan {
    /// The call is answered from the desktop's state.
    Answer(ToolOutcome),
    /// The desktop loop issues `command` like a key press would, and then answers the call with
    /// `result`.
    Issue {
        command: DesktopCommand,
        result: ToolResultJson,
    },
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

    /// Plans a call of the tool named `tool` on the MCP root `root`. Every id and slot the call
    /// names is checked before anything happens.
    pub fn call_tool(&self, root: ProjectId, tool: &str, arguments: &str) -> ToolCallPlan {
        match self.try_call_tool(root, tool, arguments) {
            Ok(plan) => plan,
            Err(ToolError { code, message }) => ToolCallPlan::Answer(ToolOutcome::Err {
                code: code.name().into(),
                message,
            }),
        }
    }

    fn try_call_tool(
        &self,
        root: ProjectId,
        tool: &str,
        arguments: &str,
    ) -> Result<ToolCallPlan, ToolError> {
        let Some((desktop_tool, declaration)) = DESKTOP_TOOLS
            .iter()
            .find(|(_, declaration)| declaration.name.is(tool))
        else {
            return Err(ToolError {
                code: ToolErrorCode::UnknownId,
                message: format!("No tool is named '{tool}'."),
            });
        };
        self.allow_scope(root, declaration.access_scope, "the tool needs it")?;
        match desktop_tool {
            DesktopTool::Describe => {
                let DescribeParams {} = decode_arguments(arguments)?;
                let description = encode_result(&self.describe_project(root));
                Ok(ToolCallPlan::Answer(ToolOutcome::Ok(description.into())))
            }
            DesktopTool::AssignSlot => self.assign_slot(root, decode_arguments(arguments)?),
            DesktopTool::ClearSlot => self.clear_slot(root, decode_arguments(arguments)?),
            DesktopTool::MoveSlot => self.move_slot(root, decode_arguments(arguments)?),
            DesktopTool::PushSlot => self.push_slot(root, decode_arguments(arguments)?),
            DesktopTool::Group => self.group(root, decode_arguments(arguments)?),
            DesktopTool::Ungroup => self.ungroup(root, decode_arguments(arguments)?),
            DesktopTool::StartInstance => self.start_instance(root, decode_arguments(arguments)?),
            DesktopTool::StopInstance => self.stop_instance(root, decode_arguments(arguments)?),
        }
    }

    fn assign_slot(
        &self,
        root: ProjectId,
        params: AssignSlotParams,
    ) -> Result<ToolCallPlan, ToolError> {
        let (parent, slot) = self.covered_slot(root, &params.slot)?;
        if let ContentSpec::Launcher(spec) = &params.content {
            let sets_params = spec
                .params
                .as_ref()
                .is_some_and(|params| !params.0.is_empty());
            match security::creating_launcher(sets_params, self.granted_access_scopes(root)) {
                CreatingLauncher::Allow => {}
                CreatingLauncher::RefuseParamsWithoutAct => {
                    return Err(scope_not_granted(
                        AccessScope::Act,
                        "a launcher's params decide what its instances run",
                    ));
                }
            }
        }
        let content = slot_assignment(params.content);
        let id = match content.content() {
            SlotContent::Launcher(launcher) => uuid::Uuid::from(launcher),
            SlotContent::Project(project) => uuid::Uuid::from(project),
        };
        let command = ConfigurationCommand::AssignSlot {
            parent: Some(parent),
            slot,
            content,
            shift: params.shift.into(),
        };
        Ok(issue(command, &CreatedContent { id: id.to_string() }))
    }

    fn clear_slot(
        &self,
        root: ProjectId,
        params: ClearSlotParams,
    ) -> Result<ToolCallPlan, ToolError> {
        let (parent, slot) = self.covered_slot(root, &params.slot)?;
        let command = ConfigurationCommand::ClearSlot {
            parent,
            slot,
            shift: params.shift.into(),
        };
        Ok(issue(command, &NoResult {}))
    }

    /// Both slots lie inside one MCP root, and MCP roots never nest, so the moved content holds no
    /// MCP root, and the move cannot nest one inside another.
    fn move_slot(
        &self,
        root: ProjectId,
        params: MoveSlotParams,
    ) -> Result<ToolCallPlan, ToolError> {
        let source = self.covered_slot(root, &params.from)?;
        let dest = self.covered_slot(root, &params.to)?;
        let command = ConfigurationCommand::MoveSlot { source, dest };
        Ok(issue(command, &NoResult {}))
    }

    fn push_slot(
        &self,
        root: ProjectId,
        params: PushSlotParams,
    ) -> Result<ToolCallPlan, ToolError> {
        let (parent, slot) = self.covered_slot(root, &params.slot)?;
        let command = ConfigurationCommand::PushSlot {
            parent,
            slot,
            direction: params.direction.into(),
        };
        Ok(issue(command, &NoResult {}))
    }

    /// The planner groups only the assigned slots among the positions, and refuses when none is.
    fn group(&self, root: ProjectId, params: GroupParams) -> Result<ToolCallPlan, ToolError> {
        let parent = self.covered_project(root, &params.project_id)?;
        let slots = params
            .positions
            .into_iter()
            .map(|position| self.matrix_slot(parent, position))
            .collect::<Result<Vec<_>, _>>()?;
        let project = ProjectId::new();
        let command = ConfigurationCommand::Group {
            parent,
            slots,
            project,
            name: params.name.into(),
            focus: FocusIntent::Keep,
        };
        let id = uuid::Uuid::from(project).to_string();
        Ok(issue(command, &CreatedContent { id }))
    }

    /// The planner refuses to ungroup the MCP root itself, whose slots would leave the covered
    /// subtree.
    fn ungroup(&self, root: ProjectId, params: UngroupParams) -> Result<ToolCallPlan, ToolError> {
        let project = self.covered_project(root, &params.project_id)?;
        let command = ConfigurationCommand::Ungroup {
            project,
            focus: FocusIntent::Keep,
        };
        Ok(issue(command, &NoResult {}))
    }

    fn start_instance(
        &self,
        root: ProjectId,
        params: StartInstanceParams,
    ) -> Result<ToolCallPlan, ToolError> {
        let launcher = self.covered_launcher(root, &params.launcher_id)?;
        let instance: InstanceId = uuid::Uuid::new_v4().into();
        let command = DesktopCommand::StartInstance {
            launcher,
            instance,
            root: None,
            parameters: self.aggregates.configuration[launcher].params.clone(),
            kind: InstanceKind::Primary,
            focus: FocusIntent::Keep,
        };
        let id = instance.to_string();
        Ok(issue(command, &CreatedContent { id }))
    }

    fn stop_instance(
        &self,
        root: ProjectId,
        params: StopInstanceParams,
    ) -> Result<ToolCallPlan, ToolError> {
        let instance = self.covered_instance(root, &params.instance_id)?;
        Ok(issue(DesktopCommand::StopInstance(instance), &NoResult {}))
    }

    /// The project that `id` names, when the MCP root `root` covers it.
    fn covered_project(&self, root: ProjectId, id: &str) -> Result<ProjectId, ToolError> {
        let configuration = &self.aggregates.configuration;
        let project = uuid::Uuid::parse_str(id)
            .ok()
            .map(ProjectId::from)
            .filter(|&project| configuration.project(project).is_some());
        let covering_mcp_root = project.and_then(|project| configuration.mcp_root_of(project));
        match security::naming_target(covering_mcp_root, root) {
            NamingTarget::Allow => Ok(project.expect("a covered project exists")),
            NamingTarget::RefuseUnknown => Err(unknown_id("project", id)),
        }
    }

    /// The launcher that `id` names, when the MCP root `root` covers it.
    fn covered_launcher(&self, root: ProjectId, id: &str) -> Result<LaunchProfileId, ToolError> {
        let configuration = &self.aggregates.configuration;
        let launcher = uuid::Uuid::parse_str(id)
            .ok()
            .map(LaunchProfileId::from)
            .filter(|&launcher| configuration.launcher(launcher).is_some());
        let covering_mcp_root = launcher
            .and_then(|launcher| configuration.project_of_launcher(launcher))
            .and_then(|project| configuration.mcp_root_of(project.id));
        match security::naming_target(covering_mcp_root, root) {
            NamingTarget::Allow => Ok(launcher.expect("a covered launcher exists")),
            NamingTarget::RefuseUnknown => Err(unknown_id("launcher", id)),
        }
    }

    /// The running instance that `id` names, when the MCP root `root` covers it.
    fn covered_instance(&self, root: ProjectId, id: &str) -> Result<InstanceId, ToolError> {
        let hierarchy = &self.aggregates.hierarchy;
        let instance = uuid::Uuid::parse_str(id)
            .ok()
            .map(InstanceId::from)
            .filter(|&instance| hierarchy.exists(&instance.into()));
        let covering_mcp_root = instance
            .map(|instance| hierarchy.project_of_target(&instance.into()))
            .and_then(|project| self.aggregates.configuration.mcp_root_of(project));
        match security::naming_target(covering_mcp_root, root) {
            NamingTarget::Allow => Ok(instance.expect("a covered instance exists")),
            NamingTarget::RefuseUnknown => Err(unknown_id("instance", id)),
        }
    }

    /// The slot that `slot` names, when the MCP root `root` covers its project.
    fn covered_slot(
        &self,
        root: ProjectId,
        slot: &SlotRef,
    ) -> Result<(ProjectId, SlotPosition), ToolError> {
        let project = self.covered_project(root, &slot.project_id)?;
        Ok((project, self.matrix_slot(project, slot.position)?))
    }

    /// `position` as a slot of `project`'s matrix, when it lies inside the matrix.
    fn matrix_slot(
        &self,
        project: ProjectId,
        position: SlotPositionValue,
    ) -> Result<SlotPosition, ToolError> {
        let slot = SlotPosition {
            column: position.column,
            row: position.row,
        };
        let matrix = self.aggregates.configuration[project].matrix_bounds();
        match security::naming_slot(slot.into(), matrix) {
            NamingSlot::Allow => Ok(slot),
            NamingSlot::RefuseOutsideMatrix => Err(invalid_argument(&format!(
                "The slot ({}, {}) lies outside the project's matrix. A slot lies at most one \
                 column right of and one row below the assigned slots.",
                slot.column, slot.row
            ))),
        }
    }

    fn allow_scope(
        &self,
        root: ProjectId,
        needed: AccessScope,
        because: &str,
    ) -> Result<(), ToolError> {
        match security::calling_tool(self.granted_access_scopes(root), needed) {
            CallingTool::Allow => Ok(()),
            CallingTool::RefuseScopeNotGranted => Err(scope_not_granted(needed, because)),
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

    fn granted_access_scopes(&self, root: ProjectId) -> &AccessScopes {
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
                    position: SlotPositionValue {
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

    fn describe_launcher(&self, id: LaunchProfileId) -> LauncherDescription {
        let launcher = &self.aggregates.configuration[id];
        LauncherDescription {
            id: uuid::Uuid::from(id).to_string(),
            name: launcher.name.clone(),
            mode: launcher.mode.into(),
            params: launcher.params.clone(),
            full_screen_mode: launcher.full_screen_mode.into(),
            instances: self
                .aggregates
                .hierarchy
                .launcher_instances(id)
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

/// The new content a `desktop_assign_slot` call describes, with a new id.
fn slot_assignment(content: ContentSpec) -> SlotAssignment {
    match content {
        ContentSpec::Launcher(spec) => SlotAssignment::Launcher {
            id: LaunchProfileId::new(),
            profile: LaunchProfile {
                name: spec.name.into(),
                mode: spec.mode.map_or(LauncherMode::Visor, Into::into),
                params: spec.params.map(Into::into).unwrap_or_default(),
                full_screen_mode: spec
                    .full_screen_mode
                    .map_or(FullScreenMode::Regular, Into::into),
            },
        },
        ContentSpec::Project(name) => SlotAssignment::Project {
            id: ProjectId::new(),
            name: name.into(),
        },
    }
}

/// Plans a call whose `command` the desktop loop carries out before it answers with `result` (see
/// `tool_host::IssuedCommand`).
fn issue(command: impl Into<DesktopCommand>, result: &impl Serialize) -> ToolCallPlan {
    ToolCallPlan::Issue {
        command: command.into(),
        result: encode_result(result),
    }
}

/// The answer for an id that names nothing the MCP root covers. It reads the same whether the
/// target does not exist or lies outside the root.
fn unknown_id(kind: &str, id: &str) -> ToolError {
    ToolError {
        code: ToolErrorCode::UnknownId,
        message: format!("No {kind} in the MCP root has the id '{id}'."),
    }
}

fn scope_not_granted(needed: AccessScope, because: &str) -> ToolError {
    ToolError {
        code: ToolErrorCode::ScopeNotGranted,
        message: format!(
            "The MCP root does not grant the access scope '{}', and {because}.",
            needed.name()
        ),
    }
}

fn invalid_argument(message: &str) -> ToolError {
    ToolError {
        code: ToolErrorCode::InvalidArgument,
        message: message.into(),
    }
}

/// The desktop's tools, declared once, because their schemas never change.
static DESKTOP_TOOLS: LazyLock<Vec<(DesktopTool, ToolDeclaration)>> = LazyLock::new(|| {
    vec![
        (
            DesktopTool::Describe,
            ToolDeclaration::new::<DescribeParams, ProjectDescription>(
                tool_name!(McpId::DESKTOP, "describe"),
                AccessScope::Observe,
                "Returns the MCP root this connection is bound to and everything it covers: its \
                 slots, nested projects, launchers, and running instances. Ids are valid only \
                 while the desktop runs.",
            ),
        ),
        (
            DesktopTool::AssignSlot,
            ToolDeclaration::new::<AssignSlotParams, CreatedContent>(
                tool_name!(McpId::DESKTOP, "assign_slot"),
                AccessScope::Organize,
                "Puts a new launcher or a new, empty project into a slot, and returns its id. \
                 Replacing assigned content with shift \"keep\" removes it and its subtree, and is \
                 refused while instances run below it.",
            ),
        ),
        (
            DesktopTool::ClearSlot,
            ToolDeclaration::new::<ClearSlotParams, NoResult>(
                tool_name!(McpId::DESKTOP, "clear_slot"),
                AccessScope::Organize,
                "Removes a slot's content and its subtree. The slot stays unassigned unless shift \
                 is \"shift\". It is refused while instances run below the content, so stop them \
                 first with desktop_stop_instance.",
            ),
        ),
        (
            DesktopTool::MoveSlot,
            ToolDeclaration::new::<MoveSlotParams, NoResult>(
                tool_name!(McpId::DESKTOP, "move_slot"),
                AccessScope::Organize,
                "Moves a slot's content to another slot, possibly in another project of the MCP \
                 project. Onto an assigned slot of the same project, it exchanges the two \
                 contents.",
            ),
        ),
        (
            DesktopTool::Group,
            ToolDeclaration::new::<GroupParams, CreatedContent>(
                tool_name!(McpId::DESKTOP, "group"),
                AccessScope::Organize,
                "Wraps assigned slots of one project into a new project, which takes the top-left \
                 slot of their bounding box, and returns its id. Unassigned slots among the \
                 positions are ignored.",
            ),
        ),
        (
            DesktopTool::Ungroup,
            ToolDeclaration::new::<UngroupParams, NoResult>(
                tool_name!(McpId::DESKTOP, "ungroup"),
                AccessScope::Organize,
                "Moves a nested project's slots into its parent's matrix and removes the project. \
                 An empty project is removed instead. The MCP root itself cannot be ungrouped.",
            ),
        ),
        (
            DesktopTool::StartInstance,
            ToolDeclaration::new::<StartInstanceParams, CreatedContent>(
                tool_name!(McpId::DESKTOP, "start_instance"),
                AccessScope::Act,
                "Starts a primary instance of a launcher with the launcher's params, and returns \
                 its id. The user's focus stays where it is.",
            ),
        ),
        (
            DesktopTool::StopInstance,
            ToolDeclaration::new::<StopInstanceParams, NoResult>(
                tool_name!(McpId::DESKTOP, "stop_instance"),
                AccessScope::Act,
                "Stops an instance and ends its application.",
            ),
        ),
        (
            DesktopTool::PushSlot,
            ToolDeclaration::new::<PushSlotParams, NoResult>(
                tool_name!(McpId::DESKTOP, "push_slot"),
                AccessScope::Organize,
                "Pushes a slot's content one slot in a direction, together with the contents in \
                 its way.",
            ),
        ),
    ]
});

#[derive(Debug, Clone, Copy)]
enum DesktopTool {
    Describe,
    AssignSlot,
    ClearSlot,
    MoveSlot,
    PushSlot,
    Group,
    Ungroup,
    StartInstance,
    StopInstance,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DescribeParams {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AssignSlotParams {
    /// Where to put the content.
    slot: SlotRef,
    /// The new launcher or project.
    content: ContentSpec,
    /// What happens to content in the way. The default is "shift", which keeps the content.
    #[serde(default = "ShiftValue::shift")]
    shift: ShiftValue,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ClearSlotParams {
    /// The slot to clear.
    slot: SlotRef,
    /// Whether the rest of the row moves into the cleared slot. The default is "keep", which moves
    /// nothing else.
    #[serde(default = "ShiftValue::keep")]
    shift: ShiftValue,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MoveSlotParams {
    /// The assigned slot whose content moves.
    from: SlotRef,
    /// Where the content goes, outside its own subtree. An assigned slot of the same project
    /// exchanges the two contents.
    to: SlotRef,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PushSlotParams {
    /// The assigned slot to push.
    slot: SlotRef,
    /// Where it moves.
    direction: DirectionValue,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GroupParams {
    /// The project whose matrix holds the slots.
    project_id: String,
    /// The slots to wrap.
    positions: Vec<SlotPositionValue>,
    /// The new project's name.
    name: TargetName,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UngroupParams {
    /// A nested project in the MCP root, not the MCP root itself.
    project_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StartInstanceParams {
    /// The launcher to start an instance of.
    launcher_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StopInstanceParams {
    /// The instance to stop.
    instance_id: String,
}

/// A slot in a project of the MCP root.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SlotRef {
    /// The project whose matrix holds the slot.
    project_id: String,
    position: SlotPositionValue,
}

/// What a tool puts into a slot. Exactly one field is set.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
enum ContentSpec {
    /// A new launcher.
    Launcher(LauncherSpec),
    /// The name of a new, empty project.
    Project(TargetName),
}

/// The launcher a tool creates.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LauncherSpec {
    /// Shown in the title bar. Names may repeat.
    name: TargetName,
    /// The default is "visor".
    mode: Option<LauncherModeValue>,
    params: Option<LauncherParams>,
    /// The default is "regular".
    full_screen_mode: Option<FullScreenModeValue>,
}

/// The name of a desktop target, such as a launcher or project, that a tool creates. Decoding
/// refuses an empty name and one longer than [`constants::tools::MAX_NAME_CHARS`], and the schema
/// states both limits, so the limit has one source. A name may contain `/`, because it is a label
/// the user picks, not a path.
#[derive(Debug, Deserialize, Into)]
#[serde(try_from = "String")]
struct TargetName(String);

impl JsonSchema for TargetName {
    fn schema_name() -> Cow<'static, str> {
        "TargetName".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "minLength": 1,
            "maxLength": constants::tools::MAX_NAME_CHARS,
        })
    }
}

impl TryFrom<String> for TargetName {
    type Error = String;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        let length = name.chars().count();
        if length == 0 || length > constants::tools::MAX_NAME_CHARS {
            return Err(format!(
                "a name has 1 to {} characters",
                constants::tools::MAX_NAME_CHARS
            ));
        }
        Ok(Self(name))
    }
}

/// The params of a launcher that a tool creates. Decoding refuses params nested deeper than
/// [`constants::configuration::MAX_PARAMS_DEPTH`], and the schema's description states the limit.
#[derive(Debug, Deserialize, Into)]
#[serde(try_from = "Params")]
struct LauncherParams(Params);

impl JsonSchema for LauncherParams {
    fn schema_name() -> Cow<'static, str> {
        "LauncherParams".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        let description = format!(
            "Passed to the application when an instance starts, such as a command to run. They \
             nest at most {} levels deep.",
            constants::configuration::MAX_PARAMS_DEPTH
        );
        json_schema!({
            "type": "object",
            "description": description,
        })
    }
}

impl TryFrom<Params> for LauncherParams {
    type Error = String;

    fn try_from(params: Params) -> Result<Self, Self::Error> {
        if when::nesting_params(params_depth(&params)) == NestingParams::RefuseTooDeep {
            return Err(format!(
                "params nest at most {} levels deep",
                constants::configuration::MAX_PARAMS_DEPTH
            ));
        }
        Ok(Self(params))
    }
}

/// How a tool treats the slots next to the one it changes. Each tool defaults to the value that
/// removes nothing and moves the least, because defaults should do less.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum ShiftValue {
    /// Assigning pushes the assigned content and its contiguous run one column right. Clearing
    /// pulls the rest of the row left.
    Shift,
    /// Assigning onto an assigned slot replaces its content, and clearing leaves the slot
    /// unassigned.
    Keep,
}

impl ShiftValue {
    fn shift() -> Self {
        Self::Shift
    }

    fn keep() -> Self {
        Self::Keep
    }
}

impl From<ShiftValue> for SlotShift {
    fn from(shift: ShiftValue) -> Self {
        match shift {
            ShiftValue::Shift => SlotShift::Shift,
            ShiftValue::Keep => SlotShift::Keep,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum DirectionValue {
    Left,
    Right,
    Up,
    Down,
}

impl From<DirectionValue> for Direction {
    fn from(direction: DirectionValue) -> Self {
        match direction {
            DirectionValue::Left => Direction::Left,
            DirectionValue::Right => Direction::Right,
            DirectionValue::Up => Direction::Up,
            DirectionValue::Down => Direction::Down,
        }
    }
}

/// The result of a tool that creates content.
#[derive(Debug, Serialize, JsonSchema)]
struct CreatedContent {
    /// The id of the new launcher, project, or instance.
    id: String,
}

/// The result of a tool that creates nothing.
#[derive(Debug, Serialize, JsonSchema)]
struct NoResult {}

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
    position: SlotPositionValue,
    launcher: Option<LauncherDescription>,
    project: Option<ProjectDescription>,
}

/// A slot's position in its project's matrix.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SlotPositionValue {
    /// Counted from 0.
    column: u32,
    /// Counted from 0.
    row: u32,
}

#[derive(Debug, Serialize, JsonSchema)]
struct LauncherDescription {
    id: String,
    name: String,
    mode: LauncherModeValue,
    /// Passed to the application when an instance starts, such as a command to run.
    params: Params,
    full_screen_mode: FullScreenModeValue,
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
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum LauncherModeValue {
    Visor,
    Band,
}

impl From<LauncherMode> for LauncherModeValue {
    fn from(mode: LauncherMode) -> Self {
        match mode {
            LauncherMode::Visor => Self::Visor,
            LauncherMode::Band => Self::Band,
        }
    }
}

impl From<LauncherModeValue> for LauncherMode {
    fn from(mode: LauncherModeValue) -> Self {
        match mode {
            LauncherModeValue::Visor => Self::Visor,
            LauncherModeValue::Band => Self::Band,
        }
    }
}

/// Whether a launcher's primary instances fill the window.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum FullScreenModeValue {
    Regular,
    Fullscreen,
}

impl From<FullScreenMode> for FullScreenModeValue {
    fn from(mode: FullScreenMode) -> Self {
        match mode {
            FullScreenMode::Regular => Self::Regular,
            FullScreenMode::FullScreen => Self::Fullscreen,
        }
    }
}

impl From<FullScreenModeValue> for FullScreenMode {
    fn from(mode: FullScreenModeValue) -> Self {
        match mode {
            FullScreenModeValue::Regular => Self::Regular,
            FullScreenModeValue::Fullscreen => Self::FullScreen,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum InstanceKindDescription {
    Primary,
    Assistant,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name beyond the limit fails while the arguments are decoded, before a tool runs.
    #[test]
    fn decoding_refuses_a_name_beyond_the_limit() {
        let arguments = |name: String| {
            serde_json::json!({
                "slot": { "project_id": "p", "position": { "column": 0, "row": 0 } },
                "content": { "project": name }
            })
            .to_string()
        };
        let longest = "n".repeat(constants::tools::MAX_NAME_CHARS);

        assert!(decode_arguments::<AssignSlotParams>(&arguments(longest.clone())).is_ok());
        let error = decode_arguments::<AssignSlotParams>(&arguments(longest + "n")).unwrap_err();
        assert_eq!(error.code, ToolErrorCode::InvalidArgument);
        assert!(decode_arguments::<AssignSlotParams>(&arguments(String::new())).is_err());
    }
}
