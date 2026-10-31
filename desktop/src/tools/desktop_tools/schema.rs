//! The types an agent sees of the desktop's tools: each tool's parameters, which decode and check
//! an agent's arguments, and each tool's result. Their doc comments become the descriptions of the
//! tools' JSON Schemas, so they are written for the agent.

use std::borrow::Cow;

use derive_more::Into;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::desktop_system::{Direction, SlotShift};
use crate::policy::constants;
use crate::policy::when::{self, NestingParams};
use crate::projects::{FullScreenMode, LauncherMode, Params, params_depth};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DescribeParams {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssignSlotParams {
    /// Where to put the content.
    pub slot: SlotRef,
    /// The new launcher or project.
    pub content: ContentSpec,
    /// What happens to content in the way. The default is "shift", which keeps the content.
    #[serde(default = "ShiftValue::shift")]
    pub shift: ShiftValue,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClearSlotParams {
    /// The slot to clear.
    pub slot: SlotRef,
    /// Whether the rest of the row moves into the cleared slot. The default is "keep", which moves
    /// nothing else.
    #[serde(default = "ShiftValue::keep")]
    pub shift: ShiftValue,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveSlotParams {
    /// The assigned slot whose content moves.
    pub from: SlotRef,
    /// Where the content goes, outside its own subtree. An assigned slot of the same project
    /// exchanges the two contents.
    pub to: SlotRef,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PushSlotParams {
    /// The assigned slot to push.
    pub slot: SlotRef,
    /// Where it moves.
    pub direction: DirectionValue,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupParams {
    /// The project whose matrix holds the slots.
    pub project_id: String,
    /// The slots to wrap.
    pub positions: Vec<SlotPositionValue>,
    /// The new project's name.
    pub name: TargetName,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UngroupParams {
    /// A nested project in the MCP root, not the MCP root itself.
    pub project_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StartInstanceParams {
    /// The launcher to start an instance of.
    pub launcher_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StopInstanceParams {
    /// The instance to stop.
    pub instance_id: String,
}

/// A slot in a project of the MCP root.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlotRef {
    /// The project whose matrix holds the slot.
    pub project_id: String,
    pub position: SlotPositionValue,
}

/// What a tool puts into a slot. Exactly one field is set.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum ContentSpec {
    /// A new launcher.
    Launcher(LauncherSpec),
    /// The name of a new, empty project.
    Project(TargetName),
}

/// The launcher a tool creates.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LauncherSpec {
    /// Shown in the title bar. Names may repeat.
    pub name: TargetName,
    /// The default is "visor".
    pub mode: Option<LauncherModeValue>,
    pub params: Option<LauncherParams>,
    /// The default is "regular".
    pub full_screen_mode: Option<FullScreenModeValue>,
}

/// The name of a desktop target, such as a launcher or project, that a tool creates. Decoding
/// refuses an empty name and one longer than [`constants::tools::MAX_NAME_CHARS`], and the schema
/// states both limits, so the limit has one source. A name may contain `/`, because it is a label
/// the user picks, not a path.
#[derive(Debug, Deserialize, Into)]
#[serde(try_from = "String")]
pub struct TargetName(String);

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
pub struct LauncherParams(Params);

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

impl LauncherParams {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
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
pub enum ShiftValue {
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
pub enum DirectionValue {
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
pub struct CreatedContent {
    /// The id of the new launcher, project, or instance.
    pub id: String,
}

/// The result of a tool that creates nothing.
#[derive(Debug, Serialize, JsonSchema)]
pub struct NoResult {}

/// A project: a matrix of slots that holds launchers and nested projects.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectDescription {
    /// Valid until the desktop quits.
    pub id: String,
    /// Names may repeat.
    pub name: String,
    /// Assigned slots only.
    pub slots: Vec<SlotDescription>,
}

/// Exactly one of `launcher` and `project` is set.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SlotDescription {
    pub position: SlotPositionValue,
    pub launcher: Option<LauncherDescription>,
    pub project: Option<ProjectDescription>,
}

/// A slot's position in its project's matrix.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlotPositionValue {
    /// Counted from 0.
    pub column: u32,
    /// Counted from 0.
    pub row: u32,
}

/// A launcher, which starts instances of its application with its params.
#[derive(Debug, Serialize, JsonSchema)]
pub struct LauncherDescription {
    /// Valid until the desktop quits.
    pub id: String,
    /// Shown in the title bar. Names may repeat.
    pub name: String,
    pub mode: LauncherModeValue,
    /// Passed to the application when an instance starts, such as a command to run.
    pub params: Params,
    pub full_screen_mode: FullScreenModeValue,
    /// The launcher's running instances.
    pub instances: Vec<InstanceDescription>,
}

/// A running instance of a launcher's application.
#[derive(Debug, Serialize, JsonSchema)]
pub struct InstanceDescription {
    /// Valid until the instance stops.
    pub id: String,
    pub kind: InstanceKindDescription,
    /// The title of the instance's primary view, empty while the application has set none.
    pub title: String,
}

/// How a launcher presents its instances.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LauncherModeValue {
    /// The instances stand on an arc around the one in front, which fills the launcher's panel.
    Visor,
    /// The instances stand side by side in a row.
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
pub enum FullScreenModeValue {
    /// The instances show at the size of the launcher's panel.
    Regular,
    /// The instances fill the window.
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

/// How an instance was started.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum InstanceKindDescription {
    /// Started with the launcher's params. All primary instances of a launcher share its Full
    /// Screen Mode.
    Primary,
    /// Started without the launcher's params, with a Full Screen Mode of its own.
    Assistant,
}

#[cfg(test)]
mod tests {
    use massive_applications::{ToolErrorCode, decode_arguments};

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
