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
    DEFAULT_NEW_LAUNCHER_NAME, DesktopConfiguration, LaunchProfile, LaunchProfileId, Launcher,
    LauncherMode, MatrixPlacement, Params, Project, ProjectId, ROOT_PROJECT_NAME, Slot,
    SlotAssignment,
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
///
/// Tags are looked up recursively: with the document flat and nesting expressed by
/// nested `project` nodes, a launcher can sit at any depth.
#[derive(Debug, Default)]
pub(super) struct NodeTags {
    projects: HashMap<ProjectId, usize>,
    launchers: HashMap<LaunchProfileId, usize>,
    next: usize,
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
        ConfigurationChange::SetStartupPath(path) => set_startup(document, path),
        ConfigurationChange::AssignSlot {
            parent,
            placement,
            assignment,
        } => match parent {
            // The root project has no node of its own — its slots are the document's
            // top-level nodes — so its creation change mirrors into nothing here.
            None => Ok(()),
            Some(parent) => assign_slot(document, tags, *parent, *placement, assignment),
        },
        ConfigurationChange::ClearSlot { parent, placement } => {
            clear_slot(document, tags, *parent, *placement)
        }
        ConfigurationChange::MoveSlot { source, dest } => move_slot(document, tags, *source, *dest),
    }
}

/// Parses the configuration out of the document, assigning fresh ids, synthesizing
/// the root project, and resolving the startup launcher by address path.
///
/// Each parsed node is tagged as it is parsed, so a later change addresses the
/// node the id was assigned to — duplicates included. The document is flat: its
/// top-level `launcher` and `project` nodes are the root project's slots, and
/// nesting is expressed by nested `project` nodes.
///
/// Migration of a file written before nesting existed is part of parsing: a
/// top-level node without a placement is one of the former flat project list, so
/// it gets `column=0, row=<document index>`, preserving the vertical order the old
/// desktop laid it out in. A migrated node keeps the placement it was given, so
/// re-deriving the tree is idempotent.
pub(super) fn parse_configuration(
    document: &mut KdlDocument,
) -> Result<(DesktopConfiguration, NodeTags)> {
    let mut tags = NodeTags::default();
    let mut startup: Option<String> = None;
    let mut root_slots = Vec::new();
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
            "launcher" => {
                migrate_placement(document, index);
                let node = &mut document.nodes_mut()[index];
                let launcher = parse_launcher(node)?;
                tags.tag_launcher(launcher.id, node);
                root_slots.push(Slot::launcher(launcher));
            }
            "project" => {
                migrate_placement(document, index);
                let node = &mut document.nodes_mut()[index];
                let placement = node_placement(node);
                let id = parse_project_node(node, &mut tags, &mut projects)?;
                tags.tag_project(id, node);
                root_slots.push(Slot::project(placement, id));
            }
            other => warn!("Ignoring unknown top-level node '{other}'"),
        }
    }

    let mut root = Project::new(ROOT_PROJECT_NAME.into(), root_slots);
    root.id = ProjectId::ROOT;
    projects.push(root);

    let configuration = DesktopConfiguration::new(projects, startup.as_deref())?;

    Ok((configuration, tags))
}

/// Gives a configuration that defines no launcher one in the root project, so the
/// session can boot (ADR 0012). The launcher enters the document as a real node,
/// so the first rewrite persists it and a re-parse finds it instead of adding a
/// second one.
pub(super) fn ensure_launcher(
    document: &mut KdlDocument,
    tags: &mut NodeTags,
    configuration: &mut DesktopConfiguration,
) {
    if configuration.has_launcher() {
        return;
    }
    warn!("Configuration defines no launcher; adding one to the root project");

    let placement = MatrixPlacement {
        column: 0,
        row: configuration
            .slots_ordered(ProjectId::ROOT)
            .iter()
            .map(|(placement, _)| placement.row)
            .max()
            .map_or(0, |row| row + 1),
    };
    let profile = LaunchProfile {
        name: DEFAULT_NEW_LAUNCHER_NAME.into(),
        mode: LauncherMode::Visor,
        params: Params::new(),
    };
    let id = LaunchProfileId::new();
    let mut node = launcher_node(
        &profile.name,
        profile.mode,
        &profile.params,
        placement,
        String::new(),
    );
    tags.tag_launcher(id, &mut node);
    document.nodes_mut().push(node);

    configuration.assign_slot(
        Some(ProjectId::ROOT),
        placement,
        SlotAssignment::Launcher { id, profile },
    );
}

fn set_startup(document: &mut KdlDocument, path: &Option<String>) -> Result<()> {
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

    if let Some(path) = path {
        let mut node = KdlNode::new("startup");
        node.push(path.as_str());
        node.set_format(fresh_node_format(leading.as_deref().unwrap_or("")));
        document.nodes_mut().insert(0, node);
    }
    Ok(())
}

/// Assigns `content` to `parent`'s slot at `placement`, replacing whatever the
/// slot held: replacing content is a clear plus an assign, never in place.
fn assign_slot(
    document: &mut KdlDocument,
    tags: &mut NodeTags,
    parent: ProjectId,
    placement: MatrixPlacement,
    content: &SlotAssignment,
) -> Result<()> {
    let host = slot_host_mut(document, tags, parent)?;
    let host_indent = host
        .nodes()
        .last()
        .and_then(|node| node.format())
        .map(|format| whitespace_leading(&format.leading))
        .unwrap_or_default();

    remove_slot_node(host, placement, tags);

    let node = match content {
        SlotAssignment::Launcher { id, profile } => {
            let mut node = launcher_node(
                &profile.name,
                profile.mode,
                &profile.params,
                placement,
                host_indent,
            );
            tags.tag_launcher(*id, &mut node);
            node
        }
        SlotAssignment::Project { id, name } => {
            let mut node = project_node(name, placement, host_indent);
            tags.tag_project(*id, &mut node);
            node
        }
    };
    host.nodes_mut().push(node);
    Ok(())
}

/// Empties `parent`'s slot at `placement`.
fn clear_slot(
    document: &mut KdlDocument,
    tags: &mut NodeTags,
    parent: ProjectId,
    placement: MatrixPlacement,
) -> Result<()> {
    let host = slot_host_mut(document, tags, parent)?;
    remove_slot_node(host, placement, tags);
    Ok(())
}

/// Moves the node assigned to the source slot to the destination slot, keeping the
/// node itself so its tag and its subtree survive.
fn move_slot(
    document: &mut KdlDocument,
    tags: &mut NodeTags,
    source: (ProjectId, MatrixPlacement),
    dest: (ProjectId, MatrixPlacement),
) -> Result<()> {
    let (source_parent, source_placement) = source;
    let (dest_parent, dest_placement) = dest;

    let source_host = slot_host_mut(document, tags, source_parent)?;
    let Some(index) = slot_child_index(source_host, source_placement) else {
        bail!("the source slot is empty");
    };
    let mut node = source_host.nodes_mut().remove(index);

    let dest_host = slot_host_mut(document, tags, dest_parent)?;
    remove_slot_node(dest_host, dest_placement, tags);
    set_placement(&mut node, dest_placement);
    dest_host.nodes_mut().push(node);
    Ok(())
}

/// Gives a top-level node without a placement the row of its document position, so
/// the former flat project list keeps the vertical order it was laid out in.
fn migrate_placement(document: &mut KdlDocument, index: usize) {
    let node = &mut document.nodes_mut()[index];
    if node.get("row").is_some() {
        return;
    }
    node.insert("row", KdlEntry::new_prop("row", index as i128));
    node.insert("column", KdlEntry::new_prop("column", 0));
}

/// Parses a `project` node and everything nested inside it, returning the new
/// project's id. The caller tags the node, so a project is tagged by its parent —
/// or by the top-level loop for a root slot.
fn parse_project_node(
    node: &mut KdlNode,
    tags: &mut NodeTags,
    projects: &mut Vec<Project>,
) -> Result<ProjectId> {
    let name = string_arg(node, "project")?.to_string();
    let mut slots = Vec::new();

    if let Some(children) = node.children_mut() {
        for child in children.nodes_mut() {
            let kind = child.name().value().to_string();
            match kind.as_str() {
                "launcher" => {
                    let launcher = parse_launcher(child)?;
                    tags.tag_launcher(launcher.id, child);
                    slots.push(Slot::launcher(launcher));
                }
                "project" => {
                    let placement = node_placement(child);
                    let id = parse_project_node(child, tags, projects)?;
                    tags.tag_project(id, child);
                    slots.push(Slot::project(placement, id));
                }
                other => warn!("Ignoring unknown node '{other}' in project '{name}'"),
            }
        }
    }

    let project = Project::new(name, slots);
    let id = project.id;
    projects.push(project);
    Ok(id)
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

fn slot_host_mut<'a>(
    document: &'a mut KdlDocument,
    tags: &NodeTags,
    parent: ProjectId,
) -> Result<&'a mut KdlDocument> {
    // The synthesized root has no node of its own, so its slots are the document's
    // top-level nodes.
    if parent == ProjectId::ROOT {
        return Ok(document);
    }
    Ok(project_node_mut(document, tags, parent)?.ensure_children())
}

/// Removes the node assigned to `placement` and untags it, so its id no longer
/// addresses a node.
fn remove_slot_node(host: &mut KdlDocument, placement: MatrixPlacement, tags: &mut NodeTags) {
    let Some(index) = slot_child_index(host, placement) else {
        return;
    };
    let node = host.nodes_mut().remove(index);
    tags.untag(&node);
}

/// The index of the child assigned to `placement` in a slot host's children.
fn slot_child_index(host: &KdlDocument, placement: MatrixPlacement) -> Option<usize> {
    host.nodes()
        .iter()
        .position(|node| node_placement(node) == placement)
}

/// The placement a slot node carries in its `column`/`row` properties, defaulting
/// to the origin as the parser does.
fn node_placement(node: &KdlNode) -> MatrixPlacement {
    let component = |key: &str| {
        node.get(key)
            .and_then(KdlValue::as_integer)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0)
    };
    MatrixPlacement {
        column: component("column"),
        row: component("row"),
    }
}

fn project_node(name: &str, placement: MatrixPlacement, indent: String) -> KdlNode {
    let mut node = KdlNode::new("project");
    node.push(name);
    node.push(KdlEntry::new_prop("column", i128::from(placement.column)));
    node.push(KdlEntry::new_prop("row", i128::from(placement.row)));
    node.set_format(fresh_node_format(&indent));
    node
}

/// The `leading` of a node without its comment lines: the whitespace-only
/// spacing to give a node appended after it (blank lines and indentation).
fn whitespace_leading(leading: &str) -> String {
    leading
        .split_inclusive('\n')
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect()
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

fn project_node_mut<'a>(
    document: &'a mut KdlDocument,
    tags: &NodeTags,
    project: ProjectId,
) -> Result<&'a mut KdlNode> {
    let tag = tags
        .project_tag(project)
        .with_context(|| format!("project {project:?} is not in the document"))?;
    let path = node_path_of(document, tag)
        .with_context(|| format!("project {project:?} has no document node"))?;
    node_at_path_mut(document, &path)
        .with_context(|| format!("project {project:?} has no document node"))
}

/// The index path from the document's top level to the node carrying `tag`,
/// descending into `project` children.
///
/// Paths, not references, are what the mutable lookups hand out: two `&mut` nodes
/// of one document cannot be held at once, but two paths can.
fn node_path_of(document: &KdlDocument, tag: usize) -> Option<Vec<usize>> {
    find_node_path(document.nodes(), tag)
}

fn node_at_path_mut<'a>(document: &'a mut KdlDocument, path: &[usize]) -> Option<&'a mut KdlNode> {
    let (&first, rest) = path.split_first()?;
    let mut node = document.nodes_mut().get_mut(first)?;
    for &index in rest {
        node = node.children_mut().as_mut()?.nodes_mut().get_mut(index)?;
    }
    Some(node)
}

fn find_node_path(nodes: &[KdlNode], tag: usize) -> Option<Vec<usize>> {
    for (index, node) in nodes.iter().enumerate() {
        if !is_slot_node(node) {
            continue;
        }
        if node.span().offset() == tag {
            return Some(vec![index]);
        }
        let Some(children) = node.children() else {
            continue;
        };
        if let Some(rest) = find_node_path(children.nodes(), tag) {
            let mut path = vec![index];
            path.extend(rest);
            return Some(path);
        }
    }
    None
}

/// The kinds of node a slot may host, and the only nodes that carry identity
/// tags. Restricting the search to these keeps a `startup` node's parse span from
/// being mistaken for a tag.
fn is_slot_node(node: &KdlNode) -> bool {
    matches!(node.name().value(), "project" | "launcher")
}

impl NodeTags {
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

    /// Forgets the id a removed node carried, so a later lookup cannot resolve it.
    fn untag(&mut self, node: &KdlNode) {
        let tag = node.span().offset();
        self.projects.retain(|_, value| *value != tag);
        self.launchers.retain(|_, value| *value != tag);
    }

    fn project_tag(&self, project: ProjectId) -> Option<usize> {
        self.projects.get(&project).copied()
    }

    fn next_tag(&mut self) -> usize {
        self.next += 1;
        self.next
    }
}
