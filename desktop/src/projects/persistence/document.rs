//! The KDL document's structure: applying configuration changes as surgical node
//! edits, and reading the configuration back out of the document.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use log::warn;

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlNodeFormat, KdlValue};
use serde_json::Value;

use super::parameters::{params_node, params_value};
use crate::desktop_system::change::ConfigurationChange;
use crate::projects::{
    DesktopConfiguration, LaunchProfileId, Launcher, LauncherMode, MatrixPlacement, Params,
    Project, ProjectId,
};

/// The built-in default configuration, used when no file exists on disk.
///
/// It is also a valid config that can be copied and edited as a starting point.
const DEFAULT_CONFIG: &str = r#"// Desktop configuration: projects, launchers, and the startup launcher.
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

/// The identity tags of the document's projects and launchers.
///
/// A tag rides in the node's `span`, a field the parser fills for diagnostics and
/// stringification never writes, so the document can address its nodes by identity
/// while the file stays byte-identical. Tags are assigned as nodes are parsed or
/// added, and the node lookups below resolve a tag back to its node.
#[derive(Debug, Default)]
pub(super) struct NodeTags {
    projects: HashMap<ProjectId, usize>,
    launchers: HashMap<LaunchProfileId, usize>,
    next: usize,
}

impl NodeTags {
    fn next_tag(&mut self) -> usize {
        self.next += 1;
        self.next
    }

    fn tag_project(&mut self, project: ProjectId, node: &mut KdlNode) {
        let tag = self.next_tag();
        node.set_span(tag);
        self.projects.insert(project, tag);
    }

    fn tag_launcher(&mut self, launcher: LaunchProfileId, node: &mut KdlNode) {
        let tag = self.next_tag();
        node.set_span(tag);
        self.launchers.insert(launcher, tag);
    }

    fn project_tag(&self, project: ProjectId) -> Option<usize> {
        self.projects.get(&project).copied()
    }

    fn launcher_tag(&self, launcher: LaunchProfileId) -> Option<usize> {
        self.launchers.get(&launcher).copied()
    }
}

/// Applies a configuration change as surgical node edits, so that user comments and
/// formatting survive byte-identical.
///
/// Names are read from the nodes themselves: a change addresses its nodes by id
/// through `tags`, so duplicate names address exactly the node they mean.
pub(super) fn apply_change(
    document: &mut KdlDocument,
    tags: &mut NodeTags,
    change: &ConfigurationChange,
) -> Result<()> {
    match change {
        ConfigurationChange::SetStartupLauncher(launcher) => {
            let name = match launcher {
                Some(id) => Some(launcher_name(document, tags, *id)?.to_string()),
                None => None,
            };
            set_startup(document, &name)
        }
        ConfigurationChange::AddProject { id, name } => add_project(document, tags, *id, name),
        ConfigurationChange::RemoveProject(project) => remove_project(document, tags, *project),
        ConfigurationChange::AddLauncher {
            project,
            id,
            profile,
            placement,
        } => add_launcher(
            document,
            tags,
            *project,
            *id,
            &profile.name,
            profile.mode,
            &profile.params,
            *placement,
        ),
        ConfigurationChange::MoveLauncher {
            launcher,
            placement,
        } => move_launcher(document, tags, *launcher, *placement),
        ConfigurationChange::RemoveLauncher(launcher) => remove_launcher(document, tags, *launcher),
    }
}

/// The launcher's name, or an error when the id has no launcher node.
fn launcher_name<'a>(
    document: &'a KdlDocument,
    tags: &NodeTags,
    launcher: LaunchProfileId,
) -> Result<&'a str> {
    let node = launcher_node_of(document, tags, launcher)
        .with_context(|| format!("launcher {launcher:?} is not in the document"))?;
    string_arg(node, "launcher")
}

/// The launcher node the id was tagged on, or an error.
fn launcher_node_of<'a>(
    document: &'a KdlDocument,
    tags: &NodeTags,
    launcher: LaunchProfileId,
) -> Result<&'a KdlNode> {
    let tag = tags
        .launcher_tag(launcher)
        .with_context(|| format!("launcher {launcher:?} is not in the document"))?;
    document
        .nodes()
        .iter()
        .filter_map(|node| node.children())
        .flat_map(|children| children.nodes())
        .find(|node| node.span().offset() == tag)
        .with_context(|| format!("launcher {launcher:?} has no document node"))
}

/// The launcher node the id was tagged on, or an error.
fn launcher_node_mut<'a>(
    document: &'a mut KdlDocument,
    tags: &NodeTags,
    launcher: LaunchProfileId,
) -> Result<&'a mut KdlNode> {
    let tag = tags
        .launcher_tag(launcher)
        .with_context(|| format!("launcher {launcher:?} is not in the document"))?;
    document
        .nodes_mut()
        .iter_mut()
        .filter_map(|node| node.children_mut().as_mut())
        .flat_map(|children| children.nodes_mut())
        .find(|node| node.span().offset() == tag)
        .with_context(|| format!("launcher {launcher:?} has no document node"))
}

fn set_startup(document: &mut KdlDocument, name: &Option<String>) -> Result<()> {
    // Remove every existing node first so `None` leaves the document without one.
    // A leading comment rides on the first node, so the fresh node inherits the
    // removed one's leading to keep comments like the file's header block.
    let leading = document
        .nodes()
        .iter()
        .find(|node| node.name().value() == "startup")
        .and_then(|node| node.format())
        .map(|format| format.leading.clone());

    document
        .nodes_mut()
        .retain(|node| node.name().value() != "startup");

    if let Some(name) = name {
        let mut node = KdlNode::new("startup");
        node.push(name.as_str());
        node.set_format(fresh_node_format(leading.as_deref().unwrap_or("")));
        document.nodes_mut().insert(0, node);
    }
    Ok(())
}

fn add_project(
    document: &mut KdlDocument,
    tags: &mut NodeTags,
    id: ProjectId,
    name: &str,
) -> Result<()> {
    // Match the spacing between the existing projects, without the comments: a
    // comment rides on the preceding node's leading and belongs to it, so
    // copying it wholesale would repeat the comment before the appended node.
    let leading = document
        .nodes()
        .iter()
        .rev()
        .find(|node| node.name().value() == "project")
        .and_then(|node| node.format())
        .map(|format| format.leading.clone())
        .map(|leading| whitespace_leading(&leading))
        .unwrap_or_default();

    let mut node = KdlNode::new("project");
    node.push(name);
    node.set_format(fresh_node_format(&leading));
    tags.tag_project(id, &mut node);
    document.nodes_mut().push(node);
    Ok(())
}

fn remove_project(
    document: &mut KdlDocument,
    tags: &mut NodeTags,
    project: ProjectId,
) -> Result<()> {
    let tag = tags
        .project_tag(project)
        .with_context(|| format!("project {project:?} is not in the document"))?;
    let index = document
        .nodes()
        .iter()
        .position(|node| node.span().offset() == tag)
        .with_context(|| format!("project {project:?} has no document node"))?;
    document.nodes_mut().remove(index);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_launcher(
    document: &mut KdlDocument,
    tags: &mut NodeTags,
    project: ProjectId,
    id: LaunchProfileId,
    name: &str,
    mode: LauncherMode,
    params: &serde_json::Map<String, Value>,
    placement: MatrixPlacement,
) -> Result<()> {
    let project_tag = tags
        .project_tag(project)
        .with_context(|| format!("project {project:?} is not in the document"))?;
    let project_node = document
        .nodes_mut()
        .iter_mut()
        .find(|node| node.span().offset() == project_tag)
        .with_context(|| format!("project {project:?} has no document node"))?;
    let project_indent = node_indent(project_node);
    let children = project_node.ensure_children();

    // As in `add_project`: keep the sibling spacing, drop the comment lines —
    // a comment above the last child belongs to that child, not to the node
    // appended after it.
    let launcher_indent = children
        .nodes()
        .last()
        .and_then(|node| node.format())
        .map(|format| whitespace_leading(&format.leading))
        .unwrap_or_else(|| project_indent + "    ");
    let mut node = launcher_node(name, mode, params, placement, launcher_indent);
    tags.tag_launcher(id, &mut node);
    children.nodes_mut().push(node);
    Ok(())
}

fn move_launcher(
    document: &mut KdlDocument,
    tags: &NodeTags,
    launcher: LaunchProfileId,
    placement: MatrixPlacement,
) -> Result<()> {
    let node = launcher_node_mut(document, tags, launcher)?;
    set_placement(node, placement);
    Ok(())
}

fn remove_launcher(
    document: &mut KdlDocument,
    tags: &mut NodeTags,
    launcher: LaunchProfileId,
) -> Result<()> {
    let tag = tags
        .launcher_tag(launcher)
        .with_context(|| format!("launcher {launcher:?} is not in the document"))?;
    for project in document.nodes_mut() {
        let Some(children) = project.children_mut().as_mut() else {
            continue;
        };
        if let Some(index) = children
            .nodes()
            .iter()
            .position(|node| node.span().offset() == tag)
        {
            children.nodes_mut().remove(index);
            tags.launchers.remove(&launcher);
            return Ok(());
        }
    }
    bail!("launcher {launcher:?} has no document node")
}

/// The `leading` of a node without its comment lines: the whitespace-only
/// spacing to give a node appended after it (blank lines and indentation).
fn whitespace_leading(leading: &str) -> String {
    leading
        .split_inclusive('\n')
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect()
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

/// Parses the configuration out of the document, assigning fresh ids and
/// resolving the startup launcher by name.
///
/// Each parsed node is tagged as it is parsed, so a later change addresses the
/// node the id was assigned to — duplicates included.
pub(super) fn parse_configuration(
    document: &mut KdlDocument,
) -> Result<(DesktopConfiguration, NodeTags)> {
    let mut tags = NodeTags::default();
    let mut startup: Option<String> = None;
    let mut projects = Vec::new();

    for index in 0..document.nodes().len() {
        let kind = document.nodes()[index].name().value().to_string();
        match kind.as_str() {
            "startup" => {
                if startup.is_some() {
                    warn!("Multiple `startup` nodes; using the first one");
                    continue;
                }
                startup = Some(string_arg(&document.nodes()[index], "startup")?.to_string());
            }
            "project" => {
                let name = string_arg(&document.nodes()[index], "project")?.to_string();
                let mut launchers = Vec::new();
                if let Some(children) = document.nodes_mut()[index].children_mut() {
                    for child in children.nodes_mut() {
                        let kind = child.name().value().to_string();
                        match kind.as_str() {
                            "launcher" => {
                                let launcher = parse_launcher(child)?;
                                tags.tag_launcher(launcher.id, child);
                                launchers.push(launcher);
                            }
                            other => {
                                warn!("Ignoring unknown node '{other}' in project '{name}'")
                            }
                        }
                    }
                }
                let project = Project::new(name, launchers);
                tags.tag_project(project.id, &mut document.nodes_mut()[index]);
                projects.push(project);
            }
            other => warn!("Ignoring unknown top-level node '{other}'"),
        }
    }

    Ok((
        DesktopConfiguration::new(projects, startup.as_deref())?,
        tags,
    ))
}

fn parse_launcher(node: &KdlNode) -> Result<Launcher> {
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
