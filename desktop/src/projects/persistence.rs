//! Persistence of the desktop configuration as a hand-editable KDL file.
//!
//! The `KdlDocument` stays parsed in memory for the whole session. Configuration
//! changes are applied as surgical node edits so that user comments and formatting
//! survive byte-identical; a moved node carries its own comments, because kdl keeps
//! them in the node's format. The edits land in memory immediately; the file is
//! written synchronously and atomically, once per transaction, so its several
//! changes persist as a single write. Write failures are logged, never propagated
//! to the UI.
//!
//! The file is keyed by name: projects and launchers get fresh IDs every startup, so
//! the document cannot reference them. Launcher names must be unique among their
//! project's siblings and project names unique document-wide — a change that would
//! introduce a duplicate is renamed by appending ` 2`, ` 3`, and so on, keeping the
//! live model aligned with the document.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use log::{debug, warn};
use serde_json::Value;

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlNodeFormat, KdlValue};

use crate::desktop_system::change::ProjectChange;
use crate::projects::{
    LaunchProfileId, LauncherMode, LauncherSpec, MatrixPlacement, ProjectConfiguration, ProjectId,
    ProjectSet, ProjectSpec,
};

/// The file name of the desktop configuration inside the projects directory.
pub(crate) const CONFIG_FILE_NAME: &str = "desktop.kdl";

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

startup "default"

project "default" {
    launcher "default" column=0 row=0
}
"#;

/// The in-memory, hand-editable configuration document. Configuration changes are
/// applied immediately, but the file is written once per [`ConfigurationDocument
/// ::flush`], so a transaction's several changes persist as a single write.
#[derive(Debug)]
pub(crate) struct ConfigurationDocument {
    document: KdlDocument,
    /// Where the configuration lives. `None` when no projects directory can be
    /// resolved; the document then only exists in memory.
    path: Option<PathBuf>,
    /// Configuration changes applied since the last flush, i.e. whether the
    /// in-memory document differs from the file on disk.
    pending: bool,
}

impl ConfigurationDocument {
    /// Loads the document from the projects directory, falling back to the built-in
    /// default when the file is missing.
    ///
    /// A file that exists but cannot be read or parsed aborts startup.
    pub(crate) fn load(projects_dir: Option<&Path>) -> Result<Self> {
        let path = projects_dir.map(|dir| dir.join(CONFIG_FILE_NAME));
        let document = match &path {
            Some(path) => match fs::read_to_string(path) {
                Ok(text) => text
                    .parse()
                    .with_context(|| format!("parsing {}", path.display()))?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    debug!(
                        "No configuration at {}, starting from defaults",
                        path.display()
                    );
                    default_document()
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("reading {}", path.display()));
                }
            },
            None => default_document(),
        };

        Ok(Self {
            document,
            path,
            pending: false,
        })
    }

    /// The configuration, resolved from the document.
    pub(crate) fn configuration(&self) -> Result<ProjectConfiguration> {
        configuration_from_document(&self.document)
    }

    /// Applies a configuration change to the in-memory document.
    ///
    /// The file is written later, by [`ConfigurationDocument::flush`]. A failed edit
    /// is returned without changing the document, so callers can report that
    /// persistence was rejected.
    pub(crate) fn apply(&mut self, change: ConfigChange) -> Result<()> {
        apply_change(&mut self.document, &change)?;
        self.pending = true;
        Ok(())
    }

    /// Writes the document once if the in-memory document differs from the file on
    /// disk.
    ///
    /// At the end of a transaction, so its several changes persist as one file
    /// write. Write failures are logged, never propagated to the UI.
    pub(crate) fn flush(&mut self) {
        if !self.pending {
            return;
        }
        self.pending = false;
        let Some(path) = &self.path else {
            return;
        };
        let text = self.document.to_string();
        if let Err(error) = atomic_write(path, &text) {
            warn!(
                "Failed to persist configuration to {}: {error:#}",
                path.display()
            );
        }
    }
}

fn default_document() -> KdlDocument {
    DEFAULT_CONFIG
        .parse()
        .expect("Built-in default configuration must parse")
}

/// Writes `text` to `path` atomically: a temp file in the same directory, then a
/// rename over the target.
fn atomic_write(path: &Path, text: &str) -> Result<()> {
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

/// Maps configuration ids to document names across a session.
///
/// The document is keyed by name, and launchers get fresh ids every session, so every
/// applied change needs its id's name resolved. Names must be unique among sibling
/// nodes: the `add_*` registrations deduplicate by appending ` 2`, ` 3`, ... and
/// return the final name, so the live model and the document stay aligned.
#[derive(Debug, Clone, Default)]
pub(crate) struct ConfigKeys {
    projects: std::collections::HashMap<ProjectId, String>,
    launchers: std::collections::HashMap<LaunchProfileId, (ProjectId, String)>,
}

impl ConfigKeys {
    /// Registers the projects and launchers loaded from the configuration file.
    ///
    /// The configuration's order matches the document's, so the loaded names are
    /// already unique; registration cannot introduce a collision.
    pub(crate) fn from_configuration(project_set: &ProjectSet) -> Self {
        let mut keys = Self::default();
        for project in &project_set.projects {
            keys.add_project_without_dedupe(project.id, &project.properties.name);
            for launcher in &project.launchers {
                keys.register_launcher(project.id, launcher.id, &launcher.profile.name);
            }
        }
        keys
    }

    /// Registers a project under a document-unique name, renaming it on collision.
    ///
    /// Returns the final name, which may differ from the given one.
    pub(crate) fn add_project(&mut self, id: ProjectId, name: &str) -> String {
        let name = dedupe(name, self.projects.values().map(String::as_str));
        self.projects.insert(id, name.clone());
        name
    }

    /// Registers a project without renaming, for entries loaded from the file.
    fn add_project_without_dedupe(&mut self, id: ProjectId, name: &str) {
        self.projects.insert(id, name.into());
    }

    /// Registers a launcher without renaming, for entries loaded from the file.
    fn register_launcher(&mut self, project: ProjectId, id: LaunchProfileId, name: &str) {
        self.launchers.insert(id, (project, name.into()));
    }

    /// Registers a launcher under a name unique among its project's siblings,
    /// renaming it on collision. Returns the launcher's owning project's (possibly
    /// renamed) and the launcher's final name.
    pub(crate) fn add_launcher(
        &mut self,
        project: ProjectId,
        id: LaunchProfileId,
        name: &str,
    ) -> Result<(String, String)> {
        let project_name = self
            .launcher_project(project)
            .with_context(|| "adding a launcher to an unregistered project")?
            .clone();
        let name = dedupe(
            name,
            self.launchers
                .values()
                .filter(move |(owner, _)| *owner == project)
                .map(|(_, name)| name.as_str()),
        );
        self.launchers.insert(id, (project, name.clone()));
        Ok((project_name, name))
    }

    /// Resolves a launcher's owning project's name and its own name.
    pub(crate) fn launcher_key(&self, id: &LaunchProfileId) -> Result<(&str, &str)> {
        let (project, name) = self
            .launchers
            .get(id)
            .with_context(|| format!("launcher {id:?} was never registered for persistence"))?;
        let project_name = self.launcher_project(*project)?;
        Ok((project_name, name.as_str()))
    }

    fn launcher_project(&self, id: ProjectId) -> Result<&String> {
        self.projects
            .get(&id)
            .with_context(|| "launcher's owning project was never registered for persistence")
    }

    /// Drops a launcher's registration, returning its owning project's name and its
    /// own name.
    pub(crate) fn remove_launcher(&mut self, id: &LaunchProfileId) -> Result<(String, String)> {
        let (project, name) = self
            .launchers
            .remove(id)
            .with_context(|| format!("launcher {id:?} was never registered for persistence"))?;
        let project_name = self
            .projects
            .get(&project)
            .with_context(|| "launcher's owning project was never registered for persistence")?
            .clone();
        Ok((project_name, name))
    }

    /// Drops a project's and its launchers' registrations, returning the project's
    /// name.
    pub(crate) fn remove_project(&mut self, id: &ProjectId) -> Result<String> {
        self.launchers.retain(|_, (project, _)| project != id);
        self.projects
            .get(id)
            .with_context(|| format!("project {id:?} was never registered for persistence"))
            .cloned()
    }

    /// Translates a configuration change from live-model ids into document terms.
    ///
    /// `startup_profile_name` decides the `startup` node's target for
    /// [`ConfigChange::SetStartup`]: the live model has already resolved the profile
    /// id (`Some`), or cleared it (`None`).
    pub(crate) fn config_change(
        &mut self,
        change: &ProjectChange,
        startup_profile_name: Option<Option<&str>>,
    ) -> Result<ConfigChange> {
        match change {
            ProjectChange::AddProject { id, properties } => {
                let name = self.add_project(*id, &properties.name);
                Ok(ConfigChange::AddProject { name })
            }
            ProjectChange::RemoveProject(project) => {
                let name = self.remove_project(project)?;
                Ok(ConfigChange::RemoveProject { name })
            }
            ProjectChange::AddLauncher {
                project,
                id,
                profile,
                placement,
            } => {
                let (project, name) = self.add_launcher(*project, *id, &profile.name)?;
                Ok(ConfigChange::AddLauncher {
                    project,
                    name,
                    mode: profile.mode,
                    params: profile.params.clone(),
                    placement: *placement,
                })
            }
            ProjectChange::MoveLauncher {
                launcher,
                placement,
            } => {
                let (project, name) = self.launcher_key(launcher)?;
                Ok(ConfigChange::MoveLauncher {
                    project: project.into(),
                    name: name.into(),
                    placement: *placement,
                })
            }
            ProjectChange::RemoveLauncher(launch_profile_id) => {
                let (project, name) = self.remove_launcher(launch_profile_id)?;
                Ok(ConfigChange::RemoveLauncher { project, name })
            }
            ProjectChange::SetStartupProfile(_) => {
                let name = startup_profile_name.flatten().map(str::to_owned);
                Ok(ConfigChange::SetStartup(name))
            }
        }
    }
}

/// Appends a numeric suffix while a name collides with any existing name.
fn dedupe<'a>(name: &str, existing: impl Iterator<Item = &'a str> + Clone) -> String {
    if !existing.clone().any(|existing| existing == name) {
        return name.into();
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{name} {suffix}");
        if !existing.clone().any(|existing| existing == candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

/// A configuration change expressed in document terms: names instead of ids.
#[derive(Debug, Clone)]
pub(crate) enum ConfigChange {
    SetStartup(Option<String>),
    AddProject {
        name: String,
    },
    RemoveProject {
        name: String,
    },
    AddLauncher {
        project: String,
        name: String,
        mode: LauncherMode,
        params: serde_json::Map<String, Value>,
        placement: MatrixPlacement,
    },
    MoveLauncher {
        project: String,
        name: String,
        placement: MatrixPlacement,
    },
    RemoveLauncher {
        project: String,
        name: String,
    },
}

fn apply_change(document: &mut KdlDocument, change: &ConfigChange) -> Result<()> {
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
    let project_node = project_node(document, project)?;
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
    let children = project_children(document, project)?;
    let launcher_index = named_index(children.nodes(), "launcher", name)
        .with_context(|| format!("Launcher '{name}' not found in project '{project}'"))?;
    set_placement(&mut children.nodes_mut()[launcher_index], placement);
    Ok(())
}

fn remove_launcher(document: &mut KdlDocument, project: &str, name: &str) -> Result<()> {
    let children = project_children(document, project)?;
    let launcher_index = named_index(children.nodes(), "launcher", name)
        .with_context(|| format!("Launcher '{name}' not found in project '{project}'"))?;
    children.nodes_mut().remove(launcher_index);
    Ok(())
}

/// The project node with the given name, or an error.
fn project_node<'a>(document: &'a mut KdlDocument, name: &str) -> Result<&'a mut KdlNode> {
    let index = named_index(document.nodes(), "project", name)
        .with_context(|| format!("Project '{name}' not found in the configuration"))?;
    Ok(&mut document.nodes_mut()[index])
}

/// The launcher nodes of the project with the given name, or an error.
fn project_children<'a>(
    document: &'a mut KdlDocument,
    name: &'a str,
) -> Result<&'a mut KdlDocument> {
    let project_node = project_node(document, name)?;
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
/// because stringification only auto-indents formatless nodes.
fn fresh_node_format(leading: &str) -> KdlNodeFormat {
    KdlNodeFormat {
        leading: leading.into(),
        terminator: "\n".into(),
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

fn params_node(key: &str, value: &Value, indent: String) -> KdlNode {
    let mut node = KdlNode::new(key);
    match value {
        Value::Null => {}
        Value::Bool(value) => node.push(*value),
        Value::Number(number) => node.push(number_value(number)),
        Value::String(value) => node.push(value.as_str()),
        Value::Array(values) => {
            for value in values {
                match value {
                    Value::String(value) => node.push(value.as_str()),
                    Value::Bool(value) => node.push(*value),
                    Value::Number(number) => node.push(number_value(number)),
                    other => warn!("Skipping unsupported parameter value for '{key}': {other}"),
                }
            }
        }
        other => warn!("Skipping unsupported parameter value for '{key}': {other}"),
    }
    node.set_format(fresh_node_format(&indent));
    node
}

fn number_value(number: &serde_json::Number) -> KdlValue {
    if let Some(integer) = number.as_i64() {
        KdlValue::from(i128::from(integer))
    } else if let Some(float) = number.as_f64() {
        KdlValue::from(float)
    } else {
        KdlValue::Null
    }
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

/// Reads the configuration back out of the document.
fn configuration_from_document(document: &KdlDocument) -> Result<ProjectConfiguration> {
    let mut startup = None;
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
                            "launcher" => launchers.push(launcher_spec(child)?),
                            other => warn!("Ignoring unknown node '{other}' in project '{name}'"),
                        }
                    }
                }
                projects.push(ProjectSpec {
                    name: name.into(),
                    launchers,
                });
            }
            other => warn!("Ignoring unknown top-level node '{other}'"),
        }
    }

    Ok(ProjectConfiguration { startup, projects })
}

fn launcher_spec(node: &KdlNode) -> Result<LauncherSpec> {
    let name = string_arg(node, "launcher")?;

    let column = placement_component(node, "column")?;
    let row = placement_component(node, "row")?;
    let mode = match node.get("mode") {
        Some(value) => mode_from_value(value)?,
        None => LauncherMode::default(),
    };

    let mut params = serde_json::Map::new();
    if let Some(children) = node.children() {
        for child in children.nodes() {
            let key = child.name().value();
            if params.insert(key.into(), params_value(child)).is_some() {
                warn!("Duplicate parameter '{key}'; using the last one");
            }
        }
    }

    Ok(LauncherSpec {
        name: name.into(),
        column,
        row,
        mode,
        params,
    })
}

fn params_value(node: &KdlNode) -> Value {
    let values: Vec<Value> = node
        .entries()
        .iter()
        .filter(|entry| entry.name().is_none())
        .map(|entry| json_value(entry.value()))
        .collect();
    match values.as_slice() {
        [] => Value::Bool(true),
        [single] => single.clone(),
        _ => Value::Array(values),
    }
}

fn json_value(value: &KdlValue) -> Value {
    match value {
        KdlValue::String(value) => Value::String(value.into()),
        KdlValue::Integer(value) => match i64::try_from(*value) {
            Ok(value) => Value::Number(value.into()),
            // Out-of-i64-range integers lose precision as JSON numbers.
            Err(_) => {
                serde_json::Number::from_f64(*value as f64).map_or(Value::Null, Value::Number)
            }
        },
        KdlValue::Float(value) => {
            serde_json::Number::from_f64(*value).map_or(Value::Null, Value::Number)
        }
        KdlValue::Bool(value) => Value::Bool(*value),
        KdlValue::Null => Value::Null,
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
