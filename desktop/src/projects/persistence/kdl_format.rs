//! The KDL format of the desktop configuration file (ADR 0037): the conversion between a
//! [`KdlDocument`] and the typed tree of [`PersistedConfiguration`], in both directions.
//!
//! Every format rule lives here: the `at` string, the omitted defaults, the `mcp` node, the spawn
//! parameter nodes, and the escaping of characters KDL forbids in its text. Reading is strict. An
//! unknown node or property fails the load with its location, because the reconcile would
//! otherwise remove it from the file.
//!
//! ```kdl
//! startup "/labs/build"
//!
//! launcher shell at="0,0"
//! project labs at="1,0" {
//!     mcp observe act
//!     launcher build at="0,0" full_screen_mode=fullscreen {
//!         command just
//!     }
//! }
//! ```

use std::fmt::Write;
use std::path::Path;

use anyhow::{Result, anyhow};
use kdl::{KdlDocument, KdlEntry, KdlEntryFormat, KdlError, KdlNode, KdlValue};
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
/// The value of [`FULL_SCREEN_MODE`] for the default mode. It is read, so a hand edit may spell
/// the default out, but never written.
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
    escape_forbidden_characters(document.nodes_mut());
    KdlDocument::parse(&document.to_string())
        .expect("An autoformatted configuration document must parse again")
}

/// The position of a slot node, or `None` for a node without a valid `at`. The reader and the
/// reconcile both use it, so they agree on which nodes share a position.
pub fn slot_position(node: &KdlNode) -> Option<(u32, u32)> {
    let (column, row) = node.get(AT)?.as_string()?.split_once(',')?;
    Some((column.trim().parse().ok()?, row.trim().parse().ok()?))
}

/// The name of a slot node: its first argument, wherever it stands among the properties. `None`
/// when the node has no string argument.
pub fn slot_name(node: &KdlNode) -> Option<&str> {
    node.entries()
        .iter()
        .find(|entry| entry.name().is_none())?
        .value()
        .as_string()
}

// Reading

/// Reads the `startup` node, whose one argument is the target path of the launcher the session
/// boots into.
fn read_startup(source: &Source, node: &KdlNode) -> Result<String> {
    let entries = Entries::of(source, node)?;
    entries.refuse_properties(source, node)?;
    refuse_children(source, node)?;
    match entries.arguments.as_slice() {
        [KdlValue::String(path)] => Ok(path.clone()),
        _ => Err(source.error(node, "`startup` takes the target path as its one argument")),
    }
}

/// Reads a `project` or `launcher` node: the slot's position and the content it hosts.
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
            (AT, _) => {
                at = Some(slot_position(node).ok_or_else(|| {
                    source.error(node, "`at` is a position like \"0,1\", as column and row")
                })?);
            }
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

/// Reads the children of a `project` node: its `mcp` node and its slots.
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

/// Reads the child nodes of a launcher as its spawn parameters, each named by its key.
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

/// Reads one param node, whose one argument is its plain value. Params hold no arrays or objects,
/// because no application reads them and KDL would need a shape for each (ADR 0037).
fn read_param(source: &Source, node: &KdlNode) -> Result<Value> {
    let entries = Entries::of(source, node)?;
    entries.refuse_properties(source, node)?;
    refuse_children(source, node)?;
    match entries.arguments.as_slice() {
        [value] => read_scalar(value).map_err(|error| source.error(node, &error)),
        _ => Err(source.error(
            node,
            "a param takes one value: a string, a number, a boolean, or #null",
        )),
    }
}

/// Converts a KDL value to the JSON value an application receives.
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

/// Fails for a node that takes no children block but has one.
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

    /// Fails for a node that takes no properties but has one.
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
        anyhow!(
            "{}: {message}",
            location(self.path, self.text, node.span().offset())
        )
    }
}

/// The error of a failed parse, with one line per diagnostic of `kdl`.
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

/// The path with the 1-based line and column of the byte `offset` in `text`.
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

/// Builds the node of a slot: a `launcher` with its params as children, or a `project` with its
/// `mcp` node and its slots as children.
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

/// Writes one param as a node named by its key, with its value as the one argument.
///
/// # Panics
///
/// When the value is an array or an object. The tools refuse such params and the file cannot
/// express them, so the aggregate never holds one (ADR 0037).
fn param_node(launcher: &str, key: &str, value: &Value) -> KdlNode {
    let value = match value {
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
        Value::Array(_) | Value::Object(_) => {
            panic!("Launcher '{launcher}' holds the param '{key}' that is not a plain value")
        }
    };
    let mut node = KdlNode::new(key);
    node.push(KdlEntry::new(value));
    node
}

/// Gives every node name and string value that holds a character KDL forbids in its text an
/// escaped form of its own. `kdl` escapes only quotes, backslashes, and a few control characters.
/// It writes the others as they are, and then cannot parse its own output.
fn escape_forbidden_characters(nodes: &mut [KdlNode]) {
    for node in nodes {
        if let Some(repr) = escaped(node.name().value()) {
            node.name_mut().set_repr(repr);
        }
        for entry in node.entries_mut() {
            if let Some(repr) = entry.value().as_string().and_then(escaped) {
                entry.set_format(KdlEntryFormat {
                    value_repr: repr,
                    leading: " ".into(),
                    ..Default::default()
                });
            }
        }
        if let Some(children) = node.children_mut() {
            escape_forbidden_characters(children.nodes_mut());
        }
    }
}

/// The quoted text of `text` with every forbidden character escaped as `\u{…}`, or `None` when it
/// holds none and `kdl` writes it correctly.
fn escaped(text: &str) -> Option<String> {
    if !text.chars().any(is_forbidden) {
        return None;
    }
    let mut repr = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => repr.push_str("\\\""),
            '\\' => repr.push_str("\\\\"),
            character if is_forbidden(character) => {
                write!(repr, "\\u{{{:x}}}", u32::from(character)).expect("Writing to a String");
            }
            character => repr.push(character),
        }
    }
    repr.push('"');
    Some(repr)
}

/// Whether KDL forbids `character` as it is inside a quoted string: control characters, which
/// include most line breaks, the other line and paragraph separators, the direction marks, and the
/// byte order mark (KDL 2.0, "Disallowed Literal Code Points" and "Newline").
fn is_forbidden(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{2028}'
                | '\u{2029}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "desktop.kdl";

    /// A param is one plain value (ADR 0037). Every other shape fails the load, because an array
    /// with one element would read back as its element.
    #[test]
    fn a_param_node_holds_exactly_one_value() -> Result<()> {
        let params = params_of("text \"a\"\nnumber 2.5\nflag #true\nnothing #null")?;
        assert_eq!(
            Value::Object(params),
            serde_json::json!({ "text": "a", "number": 2.5, "flag": true, "nothing": null })
        );
        for shape in ["none", "several 1 2", "prop key=1", "object { inner 2 }"] {
            assert!(params_of(shape).is_err(), "`{shape}` should fail");
        }
        Ok(())
    }

    /// A param written and read back keeps its value. A string with a character KDL forbids in its
    /// text is escaped, because `kdl` writes it as it is and then cannot parse it.
    #[test]
    fn params_round_trip_through_the_document() -> Result<()> {
        let params = serde_json::json!({
            "command": "printf '\u{1b}[31m' \"\\\" \u{2028} \u{feff}",
            "integer": -2,
            "float": 3.5,
            "flag": false,
            "nothing": null,
            "key with spaces and \u{1b}": "x",
        });
        let Value::Object(params) = params else {
            unreachable!("the params are an object")
        };
        let configuration = PersistedConfiguration {
            startup: Some("/shell\u{1b}".into()),
            slots: vec![PersistedSlot {
                at: (0, 0),
                content: PersistedSlotContent::Launcher(PersistedLauncher {
                    name: "shell\u{1b}".into(),
                    params,
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
