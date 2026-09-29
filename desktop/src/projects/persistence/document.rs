//! The KDL document's structure: applying configuration changes as surgical node
//! edits, and reading the configuration back out of the document.

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use log::warn;

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlNodeFormat, KdlValue};
use serde_json::Value;

use super::configuration::ConfigChange;
use super::parameters::{params_node, params_value};
use crate::projects::{
    DesktopConfiguration, Launcher, LauncherMode, MatrixPlacement, Params, Project,
};

/// The built-in default configuration, used when no file exists on disk.
///
/// It is also a valid config that can be copied and edited as a starting point.
const DEFAULT_CONFIG: &str = r#"// Desktop configuration: projects, launchers, and the startup profile.
// The terminal persists configuration changes to this file, preserving
// comments and formatting.
//
// A launcher spawns the primary application when clicked; child nodes are passed
// to it as spawn parameters, e.g. `command "ssh home"` runs `ssh home`. `mode`
// selects the launcher presentation and is `visor` unless set to `band`.

startup "Primary / Local"

project "Primary / Local" {
    launcher "Primary / Local" column=0 row=0 mode=band
}

project "default" {
    launcher "default" column=0 row=0
}
"#;

/// Parses the built-in default configuration.
pub(super) fn default_document() -> KdlDocument {
    DEFAULT_CONFIG
        .parse()
        .expect("Built-in default configuration must parse")
}

/// Writes `text` to `path` atomically: a temp file in the same directory, then a
/// rename over the target.
pub(super) fn atomic_write(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().unwrap_or_else(|| Path::new(".")))?;
    temp.write_all(text.as_bytes())?;
    temp.persist(path)
        .map_err(|error| anyhow::anyhow!("persisting {}", error.error))?;
    Ok(())
}

/// Applies a configuration change as surgical node edits, so that user comments and
/// formatting survive byte-identical.
pub(super) fn apply_change(document: &mut KdlDocument, change: &ConfigChange) -> Result<()> {
    match change {
        ConfigChange::SetStartup(name) => set_startup(document, name),
        ConfigChange::AddProject { name } => add_project(document, name),
        ConfigChange::RemoveProject { name } => remove_node(document, "project", name),
        ConfigChange::AddLauncher {
            project,
            name,
            mode,
            params,
            placement,
        } => add_launcher(document, project, name, *mode, params, *placement),
        ConfigChange::MoveLauncher {
            project,
            name,
            placement,
        } => move_launcher(document, project, name, *placement),
        ConfigChange::RemoveLauncher { project, name } => remove_launcher(document, project, name),
    }
}

fn set_startup(document: &mut KdlDocument, name: &Option<String>) -> Result<()> {
    // Remove every existing node first so `None` leaves the document without one.
    document
        .nodes_mut()
        .retain(|node| node.name().value() != "startup");

    if let Some(name) = name {
        let mut node = KdlNode::new("startup");
        node.push(name.as_str());
        node.set_format(fresh_node_format(""));
        document.nodes_mut().insert(0, node);
    }
    Ok(())
}

fn add_project(document: &mut KdlDocument, name: &str) -> Result<()> {
    // Match the spacing of the existing projects (blank lines and all).
    let leading = document
        .nodes()
        .iter()
        .rev()
        .find(|node| node.name().value() == "project")
        .and_then(|node| node.format())
        .map(|format| format.leading.clone())
        .unwrap_or_default();

    let mut node = KdlNode::new("project");
    node.push(name);
    node.set_format(fresh_node_format(&leading));
    document.nodes_mut().push(node);
    Ok(())
}

/// Removes the top-level node with the given name and first-string argument.
fn remove_node(document: &mut KdlDocument, kind: &str, name: &str) -> Result<()> {
    let index = named_index(document.nodes(), kind, name)
        .with_context(|| format!("{kind} '{name}' not found in the configuration"))?;
    document.nodes_mut().remove(index);
    Ok(())
}

fn add_launcher(
    document: &mut KdlDocument,
    project: &str,
    name: &str,
    mode: LauncherMode,
    params: &serde_json::Map<String, Value>,
    placement: MatrixPlacement,
) -> Result<()> {
    let project_node = project_node_mut(document, project)?;
    let project_indent = node_indent(project_node);
    let children = project_node.ensure_children();

    let launcher_indent = sibling_leading(children).unwrap_or_else(|| project_indent + "    ");
    children.nodes_mut().push(launcher_node(
        name,
        mode,
        params,
        placement,
        launcher_indent,
    ));
    Ok(())
}

fn move_launcher(
    document: &mut KdlDocument,
    project: &str,
    name: &str,
    placement: MatrixPlacement,
) -> Result<()> {
    let children = project_children_mut(document, project)?;
    let launcher_index = named_index(children.nodes(), "launcher", name)
        .with_context(|| format!("Launcher '{name}' not found in project '{project}'"))?;
    set_placement(&mut children.nodes_mut()[launcher_index], placement);
    Ok(())
}

fn remove_launcher(document: &mut KdlDocument, project: &str, name: &str) -> Result<()> {
    let children = project_children_mut(document, project)?;
    let launcher_index = named_index(children.nodes(), "launcher", name)
        .with_context(|| format!("Launcher '{name}' not found in project '{project}'"))?;
    children.nodes_mut().remove(launcher_index);
    Ok(())
}

/// The project node with the given name, or an error.
fn project_node_mut<'a>(document: &'a mut KdlDocument, name: &str) -> Result<&'a mut KdlNode> {
    let index = named_index(document.nodes(), "project", name)
        .with_context(|| format!("Project '{name}' not found in the configuration"))?;
    Ok(&mut document.nodes_mut()[index])
}

/// The launcher nodes of the project with the given name, or an error.
fn project_children_mut<'a>(
    document: &'a mut KdlDocument,
    name: &'a str,
) -> Result<&'a mut KdlDocument> {
    let project_node = project_node_mut(document, name)?;
    project_node
        .children_mut()
        .as_mut()
        .with_context(|| format!("Project '{name}' has no launchers"))
}

/// The index of the node with the given name and first-string argument.
fn named_index(nodes: &[KdlNode], name: &str, arg: &str) -> Option<usize> {
    nodes.iter().position(|node| {
        node.name().value() == name && node.get(0).and_then(KdlValue::as_string) == Some(arg)
    })
}

/// The `leading` formatting of the last child, so appended nodes keep the block's
/// spacing (blank lines and indentation).
fn sibling_leading(children: &KdlDocument) -> Option<String> {
    children
        .nodes()
        .last()
        .and_then(|node| node.format())
        .map(|format| format.leading.clone())
}

/// The indent of a node's own `leading`: the text after its last newline.
fn node_indent(node: &KdlNode) -> String {
    node.format()
        .map(|format| format.leading.clone())
        .map(|leading| match leading.rsplit_once('\n') {
            Some((_, indent)) => indent.to_string(),
            None => leading,
        })
        .unwrap_or_default()
}

/// Formatting for a freshly constructed node: explicit indent and line terminator,
/// because stringification only auto-indents formatless nodes. A node with an
/// explicit format needs `before_children` set for the space before its children
/// block; for childless nodes the field is never written, so it can stay " " here.
pub(super) fn fresh_node_format(leading: &str) -> KdlNodeFormat {
    KdlNodeFormat {
        leading: leading.into(),
        terminator: "\n".into(),
        before_children: " ".into(),
        ..Default::default()
    }
}

fn launcher_node(
    name: &str,
    mode: LauncherMode,
    params: &serde_json::Map<String, Value>,
    placement: MatrixPlacement,
    indent: String,
) -> KdlNode {
    let mut node = KdlNode::new("launcher");
    node.push(name);
    node.push(KdlEntry::new_prop("column", i128::from(placement.column)));
    node.push(KdlEntry::new_prop("row", i128::from(placement.row)));
    if mode != LauncherMode::Visor {
        node.push(KdlEntry::new_prop("mode", mode_name(mode)));
    }
    node.set_format(fresh_node_format(&indent));

    if !params.is_empty() {
        let child_indent = indent + "    ";
        let children = node.ensure_children();
        for (key, value) in params {
            children
                .nodes_mut()
                .push(params_node(key, value, child_indent.clone()));
        }
    }
    node
}

/// Updates the `column`/`row` properties of a launcher node in place, preserving the
/// rest of the node (name, mode, params, formatting).
fn set_placement(node: &mut KdlNode, placement: MatrixPlacement) {
    node.insert(
        "column",
        KdlEntry::new_prop("column", i128::from(placement.column)),
    );
    node.insert("row", KdlEntry::new_prop("row", i128::from(placement.row)));
}

/// Parses the configuration out of the document, minting fresh ids and
/// resolving the startup profile by name.
pub(super) fn configuration_from_document(document: &KdlDocument) -> Result<DesktopConfiguration> {
    let mut startup: Option<String> = None;
    let mut projects = Vec::new();

    for node in document.nodes() {
        match node.name().value() {
            "startup" => {
                if startup.is_some() {
                    warn!("Multiple `startup` nodes; using the first one");
                    continue;
                }
                startup = Some(string_arg(node, "startup")?.into());
            }
            "project" => {
                let name = string_arg(node, "project")?;
                let mut launchers = Vec::new();
                if let Some(children) = node.children() {
                    for child in children.nodes() {
                        match child.name().value() {
                            "launcher" => launchers.push(launcher(child)?),
                            other => warn!("Ignoring unknown node '{other}' in project '{name}'"),
                        }
                    }
                }
                projects.push(Project::new(name.into(), launchers));
            }
            other => warn!("Ignoring unknown top-level node '{other}'"),
        }
    }

    DesktopConfiguration::new(projects, startup.as_deref())
}

fn launcher(node: &KdlNode) -> Result<Launcher> {
    let name = string_arg(node, "launcher")?;

    let column = placement_component(node, "column")?;
    let row = placement_component(node, "row")?;
    let mode = match node.get("mode") {
        Some(value) => mode_from_value(value)?,
        None => LauncherMode::default(),
    };

    let mut params = Params::new();
    if let Some(children) = node.children() {
        for child in children.nodes() {
            let key = child.name().value();
            if params.insert(key.into(), params_value(child)).is_some() {
                warn!("Duplicate parameter '{key}'; using the last one");
            }
        }
    }

    Ok(Launcher::new(
        name.into(),
        mode,
        params,
        MatrixPlacement { column, row },
    ))
}

fn string_arg<'a>(node: &'a KdlNode, what: &'static str) -> Result<&'a str> {
    node.get(0)
        .and_then(KdlValue::as_string)
        .with_context(|| format!("`{what}` expects a quoted name argument"))
}

/// Reads a `key=<u32>` property, defaulting to 0 when absent.
fn placement_component(node: &KdlNode, key: &'static str) -> Result<u32> {
    let Some(value) = node.get(key) else {
        return Ok(0);
    };
    let integer = value
        .as_integer()
        .with_context(|| format!("`{key}` must be a non-negative integer"))?;
    u32::try_from(integer).with_context(|| format!("`{key}` must fit into u32"))
}

fn mode_name(mode: LauncherMode) -> &'static str {
    match mode {
        LauncherMode::Band => "band",
        LauncherMode::Visor => "visor",
    }
}

fn mode_from_value(value: &KdlValue) -> Result<LauncherMode> {
    let name = value
        .as_string()
        .with_context(|| "`mode` must be a string")?;
    match name {
        "band" => Ok(LauncherMode::Band),
        "visor" => Ok(LauncherMode::Visor),
        other => bail!("Unknown launcher mode '{other}' (expected `band` or `visor`)"),
    }
}
