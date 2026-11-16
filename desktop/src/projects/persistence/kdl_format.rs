//! The KDL format of the desktop configuration file (ADR 0037). This module converts between a
//! [`KdlDocument`] and the typed tree of [`PersistedConfiguration`], in both directions.
//!
//! Every format rule lives here. That covers the `at` string, the omitted defaults, the `mcp` node,
//! the spawn parameter nodes, and the escaping of characters KDL forbids in its text. Reading is
//! strict. An unknown node or property fails the load with its location, because the reconcile
//! would otherwise remove it from the file.
//!
//! ```kdl
//! startup "/labs/build"
//!
//! launcher shell at="0,0"
//! project labs at="1,0" {
//!     mcp observe act
//!     launcher build at="0,0" full_screen_mode=fullscreen {
//!         command just
//!         args (json)#"["--verbose", "build"]"#
//!     }
//! }
//! ```

use std::fmt::Write;
use std::path::Path;

use anyhow::{Result, anyhow};
use kdl::{KdlDocument, KdlEntry, KdlEntryFormat, KdlError, KdlNode, KdlValue};
use serde_json::{Number, Value};

use massive_applications::{AccessScopes, McpConfiguration};

use super::persisted::{PersistedConfiguration, PersistedSlot, PersistedSlots};
use crate::projects::{FullScreenMode, Params, SlotPosition};

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
/// The type annotation of a param whose string value holds JSON. Arrays and objects are written
/// this way, because KDL has no value of either kind.
const JSON: &str = "json";

/// Reads the typed tree from configuration text. Parse errors and violations of the format fail,
/// with the file path, line, and column in the message. Only tests read text in one step. The load
/// and the persist parse first, because the persist reconciles into the parsed document.
#[cfg(test)]
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
    let mut slots = PersistedSlots::new();
    for node in document.nodes() {
        match node.name().value() {
            STARTUP => {
                if startup.is_some() {
                    return Err(source.error(node, "a second `startup` node"));
                }
                startup = Some(read_startup(&source, node)?);
            }
            PROJECT | LAUNCHER => insert_slot(&source, node, &mut slots)?,
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
    nodes.extend(
        configuration
            .slots
            .iter()
            .map(|(&position, slot)| slot_node(position, slot)),
    );
    document.autoformat();
    write_string_reprs(document.nodes_mut());
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
    match entries.plain_arguments(source, node)?.as_slice() {
        [KdlValue::String(path)] => Ok(path.clone()),
        _ => Err(source.error(node, "`startup` takes the target path as its one argument")),
    }
}

/// Reads a slot node into `slots`. A second slot at the same position fails, because two slots
/// never share a position.
fn insert_slot(source: &Source, node: &KdlNode, slots: &mut PersistedSlots) -> Result<()> {
    let (position, slot) = read_slot(source, node)?;
    if slots.insert(position, slot).is_some() {
        let SlotPosition { column, row } = position;
        return Err(source.error(node, &format!("a second slot at \"{column},{row}\"")));
    }
    Ok(())
}

/// Reads a `project` or `launcher` node, which holds the slot's position and the content it hosts.
fn read_slot(source: &Source, node: &KdlNode) -> Result<(SlotPosition, PersistedSlot)> {
    let kind = node.name().value();
    let entries = Entries::of(source, node)?;
    let name = match entries.plain_arguments(source, node)?.as_slice() {
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
    let position = SlotPosition::from(
        at.ok_or_else(|| source.error(node, &format!("`{kind}` needs `at=\"column,row\"`")))?,
    );

    let slot = if kind == LAUNCHER {
        let params = match node.children() {
            Some(children) => read_params(source, children)?,
            None => Params::new(),
        };
        PersistedSlot::Launcher {
            name,
            params,
            full_screen_mode,
        }
    } else {
        read_project(source, node, name)?
    };
    Ok((position, slot))
}

/// Reads the children of a `project` node, which are its `mcp` node and its slots.
fn read_project(source: &Source, node: &KdlNode, name: String) -> Result<PersistedSlot> {
    let mut mcp = None;
    let mut slots = PersistedSlots::new();
    for child in node.iter_children() {
        match child.name().value() {
            MCP => {
                if mcp.is_some() {
                    return Err(source.error(child, "a second `mcp` node"));
                }
                mcp = Some(read_mcp(source, child)?);
            }
            PROJECT | LAUNCHER => insert_slot(source, child, &mut slots)?,
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
    Ok(PersistedSlot::Project { name, mcp, slots })
}

/// Reads an `mcp` node, whose arguments are the access scopes.
fn read_mcp(source: &Source, node: &KdlNode) -> Result<McpConfiguration> {
    let entries = Entries::of(source, node)?;
    entries.refuse_properties(source, node)?;
    refuse_children(source, node)?;
    let access_scopes = entries
        .plain_arguments(source, node)?
        .into_iter()
        .map(|value| {
            value
                .as_string()
                .ok_or_else(|| anyhow!("an access scope is a name"))
                .and_then(str::parse)
                .map_err(|error| source.error(node, &error.to_string()))
        })
        .collect::<Result<AccessScopes>>()?;
    Ok(McpConfiguration { access_scopes })
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

/// Reads one param node, whose one argument is its value. A plain value stands for itself, and a
/// string annotated `(json)` holds any JSON value, such as an array or an object (ADR 0037).
fn read_param(source: &Source, node: &KdlNode) -> Result<Value> {
    let entries = Entries::of(source, node)?;
    entries.refuse_properties(source, node)?;
    refuse_children(source, node)?;
    let [argument] = entries.arguments.as_slice() else {
        return Err(source.error(
            node,
            "a param takes one value: a string, a number, a boolean, #null, or (json)\"…\"",
        ));
    };
    match argument.ty().map(|ty| ty.value()) {
        None => read_scalar(argument.value()).map_err(|error| source.error(node, &error)),
        Some(JSON) => {
            let text = argument
                .value()
                .as_string()
                .ok_or_else(|| source.error(node, "a (json) value is a string"))?;
            serde_json::from_str(text)
                .map_err(|error| source.error(node, &format!("invalid JSON: {error}")))
        }
        Some(ty) => Err(source.error(
            node,
            &format!("unknown type annotation `({ty})`, only `(json)` is supported"),
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
    /// The arguments with their type annotations, which only a param reads.
    arguments: Vec<&'a KdlEntry>,
    properties: Vec<(&'a str, &'a KdlValue)>,
}

impl<'a> Entries<'a> {
    /// Splits the entries. A type annotation on the node or on a property fails, because nothing
    /// reads it and a persist would keep or drop it at random. A repeated property fails, because
    /// KDL lets the last one win, which a hand edit rarely means.
    fn of(source: &Source, node: &'a KdlNode) -> Result<Self> {
        if node.ty().is_some() {
            return Err(source.error(node, "type annotations are not supported"));
        }
        let mut entries = Self {
            arguments: Vec::new(),
            properties: Vec::new(),
        };
        for entry in node.entries() {
            match entry.name() {
                None => entries.arguments.push(entry),
                Some(_) if entry.ty().is_some() => {
                    return Err(source.error(node, "type annotations are not supported"));
                }
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

    /// The argument values of a node that is not a param, whose arguments take no type annotation.
    fn plain_arguments(&self, source: &Source, node: &KdlNode) -> Result<Vec<&'a KdlValue>> {
        if self
            .arguments
            .iter()
            .any(|argument| argument.ty().is_some())
        {
            return Err(source.error(node, "type annotations are not supported"));
        }
        Ok(self
            .arguments
            .iter()
            .map(|argument| argument.value())
            .collect())
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

/// Builds the node of a slot. A `launcher` holds its params as children, and a `project` holds its
/// `mcp` node and its slots.
fn slot_node(position: SlotPosition, slot: &PersistedSlot) -> KdlNode {
    match slot {
        PersistedSlot::Launcher {
            name,
            params,
            full_screen_mode,
        } => {
            let mut node = KdlNode::new(LAUNCHER);
            node.push(KdlEntry::new(name.as_str()));
            node.push(at_entry(position));
            if *full_screen_mode == FullScreenMode::FullScreen {
                node.push(KdlEntry::new_prop(FULL_SCREEN_MODE, FULLSCREEN));
            }
            if !params.is_empty() {
                let children = node.ensure_children().nodes_mut();
                children.extend(params.iter().map(|(key, value)| param_node(key, value)));
            }
            node
        }
        PersistedSlot::Project { name, mcp, slots } => {
            let mut node = KdlNode::new(PROJECT);
            node.push(KdlEntry::new(name.as_str()));
            node.push(at_entry(position));
            if mcp.is_some() || !slots.is_empty() {
                let children = node.ensure_children().nodes_mut();
                if let Some(mcp) = mcp {
                    let mut mcp_node = KdlNode::new(MCP);
                    for scope in &mcp.access_scopes {
                        mcp_node.push(KdlEntry::new(scope.name()));
                    }
                    children.push(mcp_node);
                }
                children.extend(
                    slots
                        .iter()
                        .map(|(&position, slot)| slot_node(position, slot)),
                );
            }
            node
        }
    }
}

/// The `at` property of a slot at `position`.
fn at_entry(SlotPosition { column, row }: SlotPosition) -> KdlEntry {
    KdlEntry::new_prop(AT, format!("{column},{row}"))
}

/// Writes one param as a node named by its key, with its value as the one argument. An array or an
/// object becomes its JSON text, annotated `(json)`, and [`write_string_reprs`] gives that text
/// its raw form.
fn param_node(key: &str, value: &Value) -> KdlNode {
    let entry = match value {
        Value::String(text) => KdlEntry::new(text.as_str()),
        Value::Number(number) => KdlEntry::new(match (number.as_i64(), number.as_u64()) {
            (Some(integer), _) => KdlValue::Integer(integer.into()),
            (None, Some(integer)) => KdlValue::Integer(integer.into()),
            (None, None) => KdlValue::Float(
                number
                    .as_f64()
                    .expect("A JSON number is an integer or a float"),
            ),
        }),
        Value::Bool(boolean) => KdlEntry::new(*boolean),
        Value::Null => KdlEntry::new(KdlValue::Null),
        Value::Array(_) | Value::Object(_) => {
            let mut entry = KdlEntry::new(value.to_string());
            entry.set_ty(JSON);
            entry
        }
    };
    let mut node = KdlNode::new(key);
    node.push(entry);
    node
}

/// Writes the strings that `kdl` would write wrongly or hard to read, after the autoformat, which
/// resets every written form.
///
/// A name or string value whose form from `kdl` does not read back gets a quoted form of its own.
/// `kdl` 6.7.1 writes the characters KDL forbids as they are, and it leaves strings such as `-.5`
/// unquoted, which it then cannot parse. A `(json)` value gets a raw string, so its quotes need no
/// backslashes.
fn write_string_reprs(nodes: &mut [KdlNode]) {
    for node in nodes {
        let name = node.name().value();
        if !reads_back(name) {
            let repr = quoted(name);
            node.name_mut().set_repr(repr);
        }
        for entry in node.entries_mut() {
            let Some(text) = entry.value().as_string() else {
                continue;
            };
            let is_json = entry.ty().is_some_and(|ty| ty.value() == JSON);
            let repr = if is_json && !text.chars().any(is_forbidden) {
                raw(text)
            } else if !reads_back(text) {
                quoted(text)
            } else {
                continue;
            };
            entry.set_format(KdlEntryFormat {
                value_repr: repr,
                leading: " ".into(),
                ..Default::default()
            });
        }
        if let Some(children) = node.children_mut() {
            write_string_reprs(children.nodes_mut());
        }
    }
}

/// Whether the form `kdl` writes for the string `text` parses back as `text`. Checking the result
/// catches every string `kdl` writes wrongly, without knowing its rules.
fn reads_back(text: &str) -> bool {
    let written = KdlValue::String(text.into()).to_string();
    KdlDocument::parse(&format!("node {written}")).is_ok_and(|document| {
        document.nodes()[0].entries().first().map(KdlEntry::value)
            == Some(&KdlValue::String(text.into()))
    })
}

/// The raw string of `text`, such as `#"["a"]"#`, with one `#` more than the longest run of `#`
/// that follows a quote inside it, so no quote in it ends the string. A raw string cannot escape,
/// so `text` holds no character KDL forbids.
fn raw(text: &str) -> String {
    let longest = text
        .match_indices('"')
        .map(|(index, _)| text[index + 1..].chars().take_while(|&c| c == '#').count())
        .max()
        .unwrap_or(0);
    let hashes = "#".repeat(longest + 1);
    format!("{hashes}\"{text}\"{hashes}")
}

/// The quoted form of `text`, with quotes, backslashes, and every character KDL forbids escaped.
fn quoted(text: &str) -> String {
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
    repr
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
    use std::collections::BTreeMap;

    use super::*;

    const PATH: &str = "desktop.kdl";

    /// A param is one value (ADR 0037). A plain value stands for itself, and a `(json)` string
    /// holds any JSON value. Every other shape fails the load, because KDL would need a rule for
    /// each, and a one-element array would read back as its element.
    #[test]
    fn a_param_node_holds_one_plain_or_json_value() -> Result<()> {
        let params = params_of(
            r##"
            text "a"
            number 2.5
            nothing #null
            args (json)#"["--login"]"#
            env (json)"{\"PATH\": \"/bin\"}"
            size (json)"12"
            "##,
        )?;
        assert_eq!(
            Value::Object(params),
            serde_json::json!({
                "text": "a",
                "number": 2.5,
                "nothing": null,
                "args": ["--login"],
                "env": { "PATH": "/bin" },
                "size": 12,
            })
        );
        for shape in [
            "none",
            "several 1 2",
            "prop key=1",
            "object { inner 2 }",
            "broken (json)\"[1,\"",
            "other (u8)1",
        ] {
            assert!(params_of(shape).is_err(), "`{shape}` should fail");
        }
        Ok(())
    }

    /// A param written and read back keeps its value, an array with one element included. A string
    /// that `kdl` writes wrongly is quoted, such as one with a character KDL forbids or `-.5`. JSON
    /// text with `"#` in it needs a raw string with more `#`.
    #[test]
    fn params_round_trip_through_the_document() -> Result<()> {
        let params = serde_json::json!({
            "command": "printf '\u{1b}[31m' \"\\\" \u{2028} \u{feff}",
            "integer": -2,
            "float": 3.5,
            "flag": false,
            "nothing": null,
            "key with spaces and \u{1b}": "x",
            "one": ["--login"],
            "nested": { "env": { "PATH": "/bin" }, "list": [[1], {}, []] },
            "hashes": ["\"#", "\"##"],
            "forbidden in json": ["\u{2028}"],
            "-.5": "+.0",
        });
        let Value::Object(params) = params else {
            unreachable!("the params are an object")
        };
        let configuration = PersistedConfiguration {
            startup: Some("/-.5\u{1b}".into()),
            slots: BTreeMap::from([(
                SlotPosition { column: 0, row: 0 },
                PersistedSlot::Launcher {
                    name: "-.5".into(),
                    params,
                    full_screen_mode: FullScreenMode::FullScreen,
                },
            )]),
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

    /// Two slots never share a position, so a file that places two at one fails the load.
    #[test]
    fn two_slots_at_one_position_fail() {
        let error =
            read("project p at=\"0,0\" {\n    launcher a at=\"1,0\"\n    launcher b at=\"1,0\"\n}")
                .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("desktop.kdl:3:5: a second slot at \"1,0\""),
            "{error}"
        );
        assert!(read("launcher a at=\"0,0\"\nproject p at=\"0,0\"").is_err());
    }

    /// The parser's own error displays no location, so the message carries one per diagnostic.
    #[test]
    fn a_parse_error_names_its_location() {
        let error = read("launcher shell at=0,0\n").unwrap_err();
        assert!(error.to_string().starts_with("desktop.kdl:1:"), "{error}");
    }

    /// A bare `mcp` node makes a project an MCP root that grants nothing, while a project without
    /// one is no MCP root (ADR 0036).
    #[test]
    fn a_bare_mcp_node_is_an_mcp_root_without_scopes() -> Result<()> {
        let configuration = read("project p at=\"0,0\" {\n    mcp\n}\nproject q at=\"1,0\"")?;
        let mcp: Vec<Option<AccessScopes>> = configuration
            .slots
            .values()
            .map(|slot| match slot {
                PersistedSlot::Project { mcp, .. } => {
                    mcp.as_ref().map(|mcp| mcp.access_scopes.clone())
                }
                PersistedSlot::Launcher { .. } => unreachable!("the fixture holds projects"),
            })
            .collect();
        assert_eq!(mcp, [Some(AccessScopes::new()), None]);
        Ok(())
    }

    fn read(text: &str) -> Result<PersistedConfiguration> {
        read_configuration(Path::new(PATH), text)
    }

    fn params_of(text: &str) -> Result<Params> {
        let configuration = read(&format!("launcher shell at=\"0,0\" {{\n{text}\n}}"))?;
        match configuration
            .slots
            .values()
            .next()
            .expect("the fixture is a launcher")
        {
            PersistedSlot::Launcher { params, .. } => Ok(params.clone()),
            PersistedSlot::Project { .. } => unreachable!("the fixture is a launcher"),
        }
    }
}
