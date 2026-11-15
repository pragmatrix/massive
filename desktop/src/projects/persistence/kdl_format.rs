//! The KDL format of the desktop configuration file (ADR 0037): the conversion between a
//! [`KdlDocument`] and the typed tree of [`PersistedConfiguration`], in both directions.
//!
//! Every format rule lives here: the `at` string, the omitted defaults, the `mcp` node, and the
//! shapes of spawn parameter nodes. Reading is strict. An unknown node or property fails the load
//! with its location, because the reconcile would otherwise remove it from the file.
//!
//! ```kdl
//! startup "/labs/build"
//!
//! launcher shell at="0,0"
//! project labs at="1,0" {
//!     mcp observe act
//!     launcher build at="0,0" full_screen_mode=fullscreen {
//!         command just
//!         env {
//!             PATH "/bin"
//!         }
//!     }
//! }
//! ```

use std::path::Path;

use anyhow::{Result, anyhow};
use kdl::{KdlDocument, KdlEntry, KdlError, KdlNode, KdlValue};
use serde_json::{Number, Value};

use massive_applications::AccessScopes;

use super::persisted::{
    PersistedConfiguration, PersistedLauncher, PersistedMcp, PersistedProject, PersistedSlot,
    PersistedSlotContent,
};
use crate::projects::{FullScreenMode, Params};

/// The node that names the launcher the session boots into.
const STARTUP: &str = "startup";
/// The node of a slot that hosts a nested project.
const PROJECT: &str = "project";
/// The node of a slot that hosts a launcher.
const LAUNCHER: &str = "launcher";
/// The child node of a project that makes it an MCP root (ADR 0036).
const MCP: &str = "mcp";
/// The property of a slot node that holds its position as `"column,row"`.
const AT: &str = "at";
/// The property of a launcher node that holds its Full Screen Mode (ADR 0014).
const FULL_SCREEN_MODE: &str = "full_screen_mode";
/// The only value of [`FULL_SCREEN_MODE`] that is written. The regular mode is the default and
/// stays out of the file.
const FULLSCREEN: &str = "fullscreen";
const REGULAR: &str = "regular";

/// Reads the typed tree from configuration text. Parse errors and violations of the format fail,
/// with the file path, line, and column in the message.
pub fn read_configuration(path: &Path, text: &str) -> Result<PersistedConfiguration> {
    let document = parse_document(path, text)?;
    read_document(path, text, &document)
}

/// Parses configuration text into a KDL document, without checking the format.
///
/// [`KdlError`] displays only "Failed to parse KDL document", so the message lists each of its
/// diagnostics with its location instead.
pub fn parse_document(path: &Path, text: &str) -> Result<KdlDocument> {
    KdlDocument::parse(text).map_err(|error| parse_error(path, text, &error))
}

/// Reads the typed tree from a document parsed from `text`. The text locates the errors.
pub fn read_document(
    path: &Path,
    text: &str,
    document: &KdlDocument,
) -> Result<PersistedConfiguration> {
    let source = Source { path, text };
    let mut startup = None;
    let mut slots = Vec::new();
    for node in document.nodes() {
        match node.name().value() {
            STARTUP => {
                if startup.is_some() {
                    return Err(source.error(node, "a second `startup` node"));
                }
                startup = Some(read_startup(&source, node)?);
            }
            PROJECT | LAUNCHER => slots.push(read_slot(&source, node)?),
            name => {
                return Err(source.error(
                    node,
                    &format!("unknown node `{name}`, expected `startup`, `project`, or `launcher`"),
                ));
            }
        }
    }
    Ok(PersistedConfiguration { startup, slots })
}

/// Builds the document of the typed tree, autoformatted and parsed again, so every node carries the
/// indentation of its depth as text. The reconcile inserts such nodes into the existing document.
///
/// # Panics
///
/// When a launcher's params hold an object or an array inside an array. The desktop policy refuses
/// such params wherever they enter, so the file cannot lose them (ADR 0037).
pub fn write_document(configuration: &PersistedConfiguration) -> KdlDocument {
    let mut document = KdlDocument::new();
    let nodes = document.nodes_mut();
    if let Some(startup) = &configuration.startup {
        let mut node = KdlNode::new(STARTUP);
        node.push(KdlEntry::new(startup.as_str()));
        nodes.push(node);
    }
    nodes.extend(configuration.slots.iter().map(slot_node));
    document.autoformat();
    KdlDocument::parse(&document.to_string())
        .expect("An autoformatted configuration document must parse again")
}

// Reading

fn read_startup(source: &Source, node: &KdlNode) -> Result<String> {
    let entries = Entries::of(source, node)?;
    entries.refuse_properties(source, node)?;
    refuse_children(source, node)?;
    match entries.arguments.as_slice() {
        [KdlValue::String(path)] => Ok(path.clone()),
        _ => Err(source.error(node, "`startup` takes the target path as its one argument")),
    }
}

fn read_slot(source: &Source, node: &KdlNode) -> Result<PersistedSlot> {
    let kind = node.name().value();
    let entries = Entries::of(source, node)?;
    let name = match entries.arguments.as_slice() {
        [KdlValue::String(name)] => name.clone(),
        _ => {
            return Err(source.error(
                node,
                &format!("`{kind}` takes its name as its one argument"),
            ));
        }
    };
    let mut at = None;
    let mut full_screen_mode = FullScreenMode::Regular;
    for (key, value) in &entries.properties {
        match (*key, kind) {
            (AT, _) => at = Some(read_position(source, node, value)?),
            (FULL_SCREEN_MODE, LAUNCHER) => {
                full_screen_mode = match value.as_string() {
                    Some(FULLSCREEN) => FullScreenMode::FullScreen,
                    Some(REGULAR) => FullScreenMode::Regular,
                    _ => {
                        return Err(
                            source.error(node, "`full_screen_mode` is `fullscreen` or `regular`")
                        );
                    }
                }
            }
            _ => {
                return Err(source.error(node, &format!("unknown property `{key}` of `{kind}`")));
            }
        }
    }
    let at =
        at.ok_or_else(|| source.error(node, &format!("`{kind}` needs `at=\"column,row\"`")))?;

    let content = if kind == LAUNCHER {
        let params = match node.children() {
            Some(children) => read_params(source, children)?,
            None => Params::new(),
        };
        PersistedSlotContent::Launcher(PersistedLauncher {
            name,
            params,
            full_screen_mode,
        })
    } else {
        PersistedSlotContent::Project(read_project(source, node, name)?)
    };
    Ok(PersistedSlot { at, content })
}

fn read_project(source: &Source, node: &KdlNode, name: String) -> Result<PersistedProject> {
    let mut mcp = None;
    let mut slots = Vec::new();
    for child in node.iter_children() {
        match child.name().value() {
            MCP => {
                if mcp.is_some() {
                    return Err(source.error(child, "a second `mcp` node"));
                }
                mcp = Some(read_mcp(source, child)?);
            }
            PROJECT | LAUNCHER => slots.push(read_slot(source, child)?),
            child_name => {
                return Err(source.error(
                    child,
                    &format!(
                        "unknown node `{child_name}` in a project, expected `mcp`, `project`, or \
                         `launcher`"
                    ),
                ));
            }
        }
    }
    Ok(PersistedProject { name, mcp, slots })
}

/// Reads an `mcp` node, whose arguments are the access scopes.
fn read_mcp(source: &Source, node: &KdlNode) -> Result<PersistedMcp> {
    let entries = Entries::of(source, node)?;
    entries.refuse_properties(source, node)?;
    refuse_children(source, node)?;
    let access_scopes = entries
        .arguments
        .iter()
        .map(|value| {
            value
                .as_string()
                .ok_or_else(|| anyhow!("an access scope is a name"))
                .and_then(str::parse)
                .map_err(|error| source.error(node, &error.to_string()))
        })
        .collect::<Result<AccessScopes>>()?;
    Ok(PersistedMcp { access_scopes })
}

/// Reads `"column,row"`. KDL has no pair value and rejects the unquoted forms (ADR 0037).
fn read_position(source: &Source, node: &KdlNode, value: &KdlValue) -> Result<(u32, u32)> {
    value
        .as_string()
        .and_then(|text| text.split_once(','))
        .and_then(|(column, row)| Some((column.trim().parse().ok()?, row.trim().parse().ok()?)))
        .ok_or_else(|| source.error(node, "`at` is a position like \"0,1\", as column and row"))
}

/// Reads the child nodes of a launcher or of a param object as spawn parameters, each named by its
/// key.
fn read_params(source: &Source, children: &KdlDocument) -> Result<Params> {
    let mut params = Params::new();
    for node in children.nodes() {
        let key = node.name().value();
        if params.contains_key(key) {
            return Err(source.error(node, &format!("a second param `{key}`")));
        }
        params.insert(key.to_owned(), read_param(source, node)?);
    }
    Ok(params)
}

/// Reads one param node by its shape: one argument is a scalar, none or several are an array, and a
/// children block is an object (ADR 0037).
fn read_param(source: &Source, node: &KdlNode) -> Result<Value> {
    let entries = Entries::of(source, node)?;
    entries.refuse_properties(source, node)?;
    let scalar = |value: &KdlValue| read_scalar(value).map_err(|error| source.error(node, &error));
    match (node.children(), entries.arguments.as_slice()) {
        (Some(children), []) => Ok(Value::Object(read_params(source, children)?)),
        (Some(_), _) => Err(source.error(
            node,
            "a param has either arguments or a children block, not both",
        )),
        (None, [value]) => scalar(value),
        (None, values) => values
            .iter()
            .map(|value| scalar(value))
            .collect::<Result<_>>()
            .map(Value::Array),
    }
}

fn read_scalar(value: &KdlValue) -> Result<Value, String> {
    Ok(match value {
        KdlValue::String(text) => Value::String(text.clone()),
        KdlValue::Integer(integer) => i64::try_from(*integer)
            .map(Value::from)
            .or_else(|_| u64::try_from(*integer).map(Value::from))
            .map_err(|_| format!("the integer {integer} is out of range"))?,
        KdlValue::Float(float) => Number::from_f64(*float)
            .map(Value::Number)
            .ok_or_else(|| format!("the number {float} is not finite"))?,
        KdlValue::Bool(boolean) => Value::Bool(*boolean),
        KdlValue::Null => Value::Null,
    })
}

fn refuse_children(source: &Source, node: &KdlNode) -> Result<()> {
    if node.children().is_some() {
        return Err(source.error(
            node,
            &format!("`{}` takes no children block", node.name().value()),
        ));
    }
    Ok(())
}

/// A node's entries, split into arguments and properties.
struct Entries<'a> {
    arguments: Vec<&'a KdlValue>,
    properties: Vec<(&'a str, &'a KdlValue)>,
}

impl<'a> Entries<'a> {
    /// Splits the entries. Type annotations fail, because nothing in the format reads them and a
    /// persist would keep or drop them at random. A repeated property fails, because KDL lets the
    /// last one win, which a hand edit rarely means.
    fn of(source: &Source, node: &'a KdlNode) -> Result<Self> {
        if node.ty().is_some() {
            return Err(source.error(node, "type annotations are not supported"));
        }
        let mut entries = Self {
            arguments: Vec::new(),
            properties: Vec::new(),
        };
        for entry in node.entries() {
            if entry.ty().is_some() {
                return Err(source.error(node, "type annotations are not supported"));
            }
            match entry.name() {
                None => entries.arguments.push(entry.value()),
                Some(key) => {
                    let key = key.value();
                    if entries.properties.iter().any(|(seen, _)| *seen == key) {
                        return Err(source.error(node, &format!("a second property `{key}`")));
                    }
                    entries.properties.push((key, entry.value()));
                }
            }
        }
        Ok(entries)
    }

    fn refuse_properties(&self, source: &Source, node: &KdlNode) -> Result<()> {
        if let Some((key, _)) = self.properties.first() {
            return Err(source.error(
                node,
                &format!("unknown property `{key}` of `{}`", node.name().value()),
            ));
        }
        Ok(())
    }
}

/// The text a document was parsed from, for error messages that point into the file.
struct Source<'a> {
    path: &'a Path,
    text: &'a str,
}

impl Source<'_> {
    fn error(&self, node: &KdlNode, message: &str) -> anyhow::Error {
        anyhow!("{}: {message}", self.location(node.span().offset()))
    }

    /// The path with the 1-based line and column of the byte `offset`.
    fn location(&self, offset: usize) -> String {
        location(self.path, self.text, offset)
    }
}

fn parse_error(path: &Path, text: &str, error: &KdlError) -> anyhow::Error {
    let messages: Vec<String> = error
        .diagnostics
        .iter()
        .map(|diagnostic| {
            let message = diagnostic
                .message
                .as_deref()
                .or(diagnostic.label.as_deref())
                .unwrap_or("invalid KDL");
            let help = diagnostic
                .help
                .as_deref()
                .map(|help| format!(" ({help})"))
                .unwrap_or_default();
            format!(
                "{}: {message}{help}",
                location(path, text, diagnostic.span.offset())
            )
        })
        .collect();
    if messages.is_empty() {
        return anyhow!("{}: {error}", path.display());
    }
    anyhow!("{}", messages.join("\n"))
}

fn location(path: &Path, text: &str, offset: usize) -> String {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit_once('\n')
        .map_or(before, |(_, last)| last)
        .chars()
        .count()
        + 1;
    format!("{}:{line}:{column}", path.display())
}

// Writing

fn slot_node(slot: &PersistedSlot) -> KdlNode {
    let (column, row) = slot.at;
    let at = KdlEntry::new_prop(AT, format!("{column},{row}"));
    match &slot.content {
        PersistedSlotContent::Launcher(launcher) => {
            let mut node = KdlNode::new(LAUNCHER);
            node.push(KdlEntry::new(launcher.name.as_str()));
            node.push(at);
            if launcher.full_screen_mode == FullScreenMode::FullScreen {
                node.push(KdlEntry::new_prop(FULL_SCREEN_MODE, FULLSCREEN));
            }
            if !launcher.params.is_empty() {
                let children = node.ensure_children().nodes_mut();
                children.extend(
                    launcher
                        .params
                        .iter()
                        .map(|(key, value)| param_node(&launcher.name, key, value)),
                );
            }
            node
        }
        PersistedSlotContent::Project(project) => {
            let mut node = KdlNode::new(PROJECT);
            node.push(KdlEntry::new(project.name.as_str()));
            node.push(at);
            if project.mcp.is_some() || !project.slots.is_empty() {
                let children = node.ensure_children().nodes_mut();
                if let Some(mcp) = &project.mcp {
                    let mut mcp_node = KdlNode::new(MCP);
                    for scope in &mcp.access_scopes {
                        mcp_node.push(KdlEntry::new(scope.name()));
                    }
                    children.push(mcp_node);
                }
                children.extend(project.slots.iter().map(slot_node));
            }
            node
        }
    }
}

/// Writes one param as a node named by its key. An object becomes a children block, even when it
/// is empty, so it reads back as an object.
fn param_node(launcher: &str, key: &str, value: &Value) -> KdlNode {
    let mut node = KdlNode::new(key);
    match value {
        Value::Object(object) => {
            let children = node.ensure_children().nodes_mut();
            children.extend(
                object
                    .iter()
                    .map(|(key, value)| param_node(launcher, key, value)),
            );
        }
        Value::Array(values) => {
            for value in values {
                let scalar = scalar_value(value).unwrap_or_else(|| {
                    panic!(
                        "Launcher '{launcher}' holds an object or an array inside the array param \
                         '{key}', which the policy refuses"
                    )
                });
                node.push(KdlEntry::new(scalar));
            }
        }
        scalar => node.push(KdlEntry::new(
            scalar_value(scalar).expect("Objects and arrays are handled above"),
        )),
    }
    node
}

/// The KDL value of a JSON scalar, or `None` for an object or an array.
fn scalar_value(value: &Value) -> Option<KdlValue> {
    Some(match value {
        Value::String(text) => KdlValue::String(text.clone()),
        Value::Number(number) => match (number.as_i64(), number.as_u64()) {
            (Some(integer), _) => KdlValue::Integer(integer.into()),
            (None, Some(integer)) => KdlValue::Integer(integer.into()),
            (None, None) => KdlValue::Float(
                number
                    .as_f64()
                    .expect("A JSON number is an integer or a float"),
            ),
        },
        Value::Bool(boolean) => KdlValue::Bool(*boolean),
        Value::Null => KdlValue::Null,
        Value::Array(_) | Value::Object(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "desktop.kdl";

    /// The shape of a param node decides its JSON value (ADR 0037).
    #[test]
    fn a_param_node_maps_by_its_shape() -> Result<()> {
        let params = params_of(
            r#"
            one "a"
            several 1 2.5 #true #null
            none
            object {
                inner "b"
            }
            empty {}
            "#,
        )?;
        assert_eq!(
            Value::Object(params),
            serde_json::json!({
                "one": "a",
                "several": [1, 2.5, true, null],
                "none": [],
                "object": { "inner": "b" },
                "empty": {},
            })
        );
        Ok(())
    }

    #[test]
    fn a_param_node_with_arguments_and_children_fails() {
        let error = params_of("mixed 1 { inner 2 }").unwrap_err();
        assert!(error.to_string().contains("not both"), "{error}");
        assert!(params_of("prop key=1").is_err());
    }

    /// A param written and read back keeps its JSON value, including the empty object, which only
    /// a children block keeps apart from an empty array.
    #[test]
    fn params_round_trip_through_the_document() -> Result<()> {
        let params = serde_json::json!({
            "command": "ssh fs110",
            "numbers": [1, -2, 3.5],
            "flag": false,
            "env": { "PATH": "/bin", "nested": { "deep": 1 } },
            "empty": {},
            "none": [],
            "key with spaces": "x",
        });
        let Value::Object(params) = params else {
            unreachable!()
        };
        let configuration = PersistedConfiguration {
            startup: None,
            slots: vec![PersistedSlot {
                at: (0, 0),
                content: PersistedSlotContent::Launcher(PersistedLauncher {
                    name: "shell".into(),
                    params: params.clone(),
                    full_screen_mode: FullScreenMode::FullScreen,
                }),
            }],
        };
        let text = write_document(&configuration).to_string();
        assert_eq!(read(&text)?, configuration, "{text}");
        Ok(())
    }

    /// An unknown node fails the load, because the reconcile would remove it from the file. The
    /// error names the node and its line.
    #[test]
    fn an_unknown_node_or_property_fails_with_its_location() {
        let error = read("startup \"/shell\"\nlauncher shell at=\"0,0\"\nwindow 1\n").unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("desktop.kdl:3:1: unknown node `window`"),
            "{error}"
        );
        let error =
            read("project p at=\"0,0\" {\n    launcher s at=\"0,0\" color=red\n}").unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("desktop.kdl:2:5: unknown property `color`"),
            "{error}"
        );
    }

    #[test]
    fn a_malformed_position_fails() {
        for at in ["0", "a,1", "0,-1", "0,1,2"] {
            let text = format!("launcher shell at=\"{at}\"");
            assert!(read(&text).is_err(), "{at} should fail");
        }
    }

    /// The parser's own error displays no location, so the message carries one per diagnostic.
    #[test]
    fn a_parse_error_names_its_location() {
        let error = read("launcher shell at=0,0\n").unwrap_err();
        assert!(error.to_string().starts_with("desktop.kdl:1:"), "{error}");
    }

    #[test]
    fn an_mcp_node_holds_the_access_scopes() -> Result<()> {
        use massive_applications::AccessScope;

        let configuration = read(
            "project p at=\"0,0\" {\n    mcp observe act\n}\nproject q at=\"1,0\" {\n    mcp\n}",
        )?;
        let scopes: Vec<Option<AccessScopes>> = configuration
            .slots
            .iter()
            .map(|slot| match &slot.content {
                PersistedSlotContent::Project(project) => {
                    project.mcp.as_ref().map(|mcp| mcp.access_scopes.clone())
                }
                PersistedSlotContent::Launcher(_) => None,
            })
            .collect();
        assert_eq!(
            scopes,
            [
                Some([AccessScope::Observe, AccessScope::Act].into()),
                Some(AccessScopes::new())
            ]
        );
        assert!(read("project p at=\"0,0\" {\n    mcp everything\n}").is_err());
        Ok(())
    }

    fn read(text: &str) -> Result<PersistedConfiguration> {
        read_configuration(Path::new(PATH), text)
    }

    fn params_of(text: &str) -> Result<Params> {
        let configuration = read(&format!("launcher shell at=\"0,0\" {{\n{text}\n}}"))?;
        match &configuration.slots[0].content {
            PersistedSlotContent::Launcher(launcher) => Ok(launcher.params.clone()),
            PersistedSlotContent::Project(_) => unreachable!("the fixture is a launcher"),
        }
    }
}
