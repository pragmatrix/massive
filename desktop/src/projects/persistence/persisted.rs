//! The desktop configuration document: the JSON file the aggregate is persisted
//! to and loaded from.
//!
//! The document is the fractal view of the configuration (ADR 0011): a project
//! node is `name` plus `slots`, the document root is the one node whose name is
//! implied (the terminal synthesizes the root project `Projects`), and nesting is
//! structural — a slot hosts either a launcher or a nested project node. Ids are
//! not serialized: projects and launchers get fresh ids at load, and the document
//! addresses content by nesting and placement alone.
//!
//! ```json
//! {
//!   "startup": "/labs/shell",
//!   "slots": [
//!     { "at": [0, 0], "launcher": { "name": "shell", "mode": "visor" } },
//!     {
//!       "at": [1, 0],
//!       "project": {
//!         "name": "labs",
//!         "slots": [
//!           { "at": [0, 0], "launcher": { "name": "build", "mode": "band", "params": { "command": "just" } } }
//!         ]
//!       }
//!     }
//!   ]
//! }
//! ```
//!
//! `at` is `[column, row]`. `mode` is `visor` or `band`, always written; `params`
//! carries the launcher's spawn parameters and is omitted when empty. `startup`
//! is the address path of the launcher the session boots into, omitted when none
//! is set.

use log::warn;
use serde::{Deserialize, Serialize};

use crate::projects::{
    DEFAULT_NEW_LAUNCHER_NAME, LaunchProfile, LaunchProfileId, Launcher, LauncherMode,
    MatrixPlacement, Params, Project, ProjectId, ROOT_PROJECT_NAME, RuntimeConfiguration, Slot,
    SlotAssignment, SlotPayload,
};

/// The document form of [`crate::projects::DesktopConfiguration`]: the startup
/// address path and the root project's slots, with every nested project hanging
/// off the slot that hosts it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedConfiguration {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup: Option<String>,
    pub slots: Vec<PersistedSlot>,
}

/// One assigned slot of a project node: its placement plus exactly one content
/// variant, whose name is the JSON key. A slot exists only while it has content,
/// so empty slots are never serialized.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedSlot {
    /// The placement as `[column, row]`.
    pub at: (u32, u32),
    #[serde(flatten)]
    pub content: PersistedSlotContent,
}

/// What a slot hosts: a launcher or a nested project, never both. The variant
/// name is the JSON key (`launcher` / `project`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PersistedSlotContent {
    Launcher(PersistedLauncher),
    Project(PersistedProject),
}

/// A launcher's spawnable profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedLauncher {
    pub name: String,
    pub mode: LauncherMode,
    #[serde(default, skip_serializing_if = "Params::is_empty")]
    pub params: Params,
}

/// A project node: the fractal unit of the document. The name is required — the
/// document root is the one node without a `project` wrapper, so it carries no
/// name and the terminal synthesizes [`ROOT_PROJECT_NAME`] for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedProject {
    pub name: String,
    pub slots: Vec<PersistedSlot>,
}

impl PersistedConfiguration {
    /// The built-in default configuration, written when no file exists. It is
    /// also a valid config that can be copied and edited as a starting point.
    pub fn default_document() -> Self {
        serde_json::from_str(DEFAULT_CONFIG).expect("Built-in default configuration must parse")
    }

    /// Derives the aggregate from the document: a depth-first walk that gives
    /// every project node a fresh id and links children through their slots,
    /// then hands the flat project list to the configuration constructor, which
    /// resolves the startup path and synthesizes the root.
    ///
    /// A configuration that defines no launcher is given one in the root project
    /// (ADR 0012): mode `visor`, no spawn parameters, and the first free
    /// placement of the root matrix. The launcher is part of the aggregate, so
    /// the first persist writes it and a re-load finds it instead of adding a
    /// second one.
    pub fn into_configuration(self) -> RuntimeConfiguration {
        let mut projects = Vec::new();
        let root_slots = persisted_slots(&self.slots, &mut projects);

        let mut root = Project::new(ROOT_PROJECT_NAME.into(), root_slots);
        root.id = ProjectId::ROOT;
        projects.push(root);

        let mut configuration = RuntimeConfiguration::new(projects, self.startup.as_deref());
        ensure_launcher(&mut configuration);
        configuration
    }

    /// Builds the document from the aggregate: a walk from the root through the
    /// slot links, rebuilding the tree. The aggregate keeps every project's
    /// slots in placement order, so the output is deterministic. The walk reads
    /// only what the id-link invariant guarantees, so it cannot fail.
    pub fn from_configuration(configuration: &RuntimeConfiguration) -> Self {
        Self {
            startup: configuration.startup_path().map(str::to_owned),
            slots: project_slots(configuration, ProjectId::ROOT),
        }
    }
}
/// The built-in default configuration, parsed by
/// [`PersistedConfiguration::default_document`].
/// Hand-editable: copy it as a starting point for a custom configuration.
const DEFAULT_CONFIG: &str = r#"{
  "startup": "/Primary / Local/Primary / Local",
  "slots": [
    {
      "at": [0, 0],
      "project": {
        "name": "Primary / Local",
        "slots": [
          { "at": [0, 0], "launcher": { "name": "Primary / Local", "mode": "band" } }
        ]
      }
    },
    {
      "at": [0, 1],
      "project": {
        "name": "default",
        "slots": [
          { "at": [0, 0], "launcher": { "name": "default", "mode": "visor" } }
        ]
      }
    }
  ]
}"#;
/// Parses a project node's slots, pushing every nested project into `projects`
/// before the project hosting it — the order the old parse produced, so
/// document-order lookups keep their semantics.
fn persisted_slots(slots: &[PersistedSlot], projects: &mut Vec<Project>) -> Vec<Slot> {
    let mut parsed: Vec<Slot> = Vec::new();
    for slot in slots {
        let placement = MatrixPlacement::from(slot.at);
        match &slot.content {
            PersistedSlotContent::Launcher(launcher) => parsed.push(Slot::launcher(
                placement,
                Launcher::new(
                    launcher.name.clone(),
                    launcher.mode,
                    launcher.params.clone(),
                ),
            )),
            PersistedSlotContent::Project(project) => {
                let nested = persisted_slots(&project.slots, projects);
                let project = Project::new(project.name.clone(), nested);
                let id = project.id;
                projects.push(project);
                parsed.push(Slot::project(placement, id));
            }
        }
    }
    parsed
}

/// Gives a configuration that defines no launcher one in the root project, so
/// the session can boot (ADR 0012). The launcher is part of the aggregate, so
/// the first persist writes it and a re-load finds it instead of adding a
/// second one.
fn ensure_launcher(configuration: &mut RuntimeConfiguration) {
    if configuration.has_launcher() {
        return;
    }
    warn!("Configuration defines no launcher; adding one to the root project");

    let placement = MatrixPlacement {
        column: 0,
        row: configuration
            .slots_ordered(ProjectId::ROOT)
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
    configuration.assign_slot(
        Some(ProjectId::ROOT),
        placement,
        SlotAssignment::Launcher { id, profile },
    );
}

/// Builds one project node's slots from the aggregate, recursing into nested
/// projects exactly once each. The launcher record rides in the slot payload and
/// every nested project exists by the id-link invariant, so lookups cannot fail:
/// a missing one panics through the `Index` impls instead of dropping content.
fn project_slots(configuration: &RuntimeConfiguration, project: ProjectId) -> Vec<PersistedSlot> {
    configuration[project]
        .slots()
        .iter()
        .map(|slot| PersistedSlot {
            at: (slot.placement.column, slot.placement.row),
            content: match &slot.content {
                SlotPayload::Launcher(launcher) => {
                    PersistedSlotContent::Launcher(PersistedLauncher {
                        name: launcher.name.clone(),
                        mode: launcher.mode,
                        params: launcher.params.clone(),
                    })
                }
                SlotPayload::Project(child) => PersistedSlotContent::Project(PersistedProject {
                    name: configuration[*child].name.clone(),
                    slots: project_slots(configuration, *child),
                }),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use serde_json::Value;

    #[test]
    fn a_document_serializes_to_the_fractal_shape() {
        let document = PersistedConfiguration {
            startup: Some("/labs/shell".into()),
            slots: vec![
                PersistedSlot {
                    at: (0, 0),
                    content: PersistedSlotContent::Launcher(PersistedLauncher {
                        name: "shell".into(),
                        mode: LauncherMode::Visor,
                        params: Params::new(),
                    }),
                },
                PersistedSlot {
                    at: (1, 0),
                    content: PersistedSlotContent::Project(PersistedProject {
                        name: "labs".into(),
                        slots: vec![PersistedSlot {
                            at: (0, 0),
                            content: PersistedSlotContent::Launcher(PersistedLauncher {
                                name: "build".into(),
                                mode: LauncherMode::Band,
                                params: [("command".into(), Value::String("just".into()))]
                                    .into_iter()
                                    .collect(),
                            }),
                        }],
                    }),
                },
            ],
        };

        let text = serde_json::to_string(&document).unwrap();
        assert_eq!(
            text,
            r#"{"startup":"/labs/shell","slots":[{"at":[0,0],"launcher":{"name":"shell","mode":"visor"}},{"at":[1,0],"project":{"name":"labs","slots":[{"at":[0,0],"launcher":{"name":"build","mode":"band","params":{"command":"just"}}}]}}]}"#
        );
    }

    #[test]
    fn a_document_round_trips_through_json() {
        let text = r#"{ "startup": "/labs/shell", "slots": [ { "at": [1, 0], "project": { "name": "labs", "slots": [] } } ] }"#;
        let document: PersistedConfiguration = serde_json::from_str(text).unwrap();

        assert_eq!(document.startup.as_deref(), Some("/labs/shell"));
        assert_eq!(
            serde_json::to_string(&document).unwrap(),
            r#"{"startup":"/labs/shell","slots":[{"at":[1,0],"project":{"name":"labs","slots":[]}}]}"#
        );
    }

    /// The document is derived from the aggregate and back without loss: the
    /// nesting, placements, launcher profiles, and the startup path survive.
    #[test]
    fn the_aggregate_round_trips_through_the_document() -> Result<()> {
        let text = r#"{
            "startup": "/labs/build",
            "slots": [
                { "at": [0, 0], "launcher": { "name": "shell", "mode": "visor" } },
                { "at": [1, 0], "project": { "name": "labs", "slots": [
                    { "at": [0, 0], "launcher": { "name": "build", "mode": "band", "params": { "command": "just" } } }
                ] } }
            ]
        }"#;
        let configuration =
            serde_json::from_str::<PersistedConfiguration>(text)?.into_configuration();

        let written =
            serde_json::to_string(&PersistedConfiguration::from_configuration(&configuration))?;
        let reloaded =
            serde_json::from_str::<PersistedConfiguration>(&written)?.into_configuration();

        assert_eq!(reloaded.startup_path(), configuration.startup_path());
        assert_eq!(reloaded.launcher_count(), configuration.launcher_count());
        let root = ProjectId::ROOT;
        let labs = reloaded
            .child_project_at(root, MatrixPlacement { column: 1, row: 0 })
            .expect("labs is a root slot");
        assert_eq!(
            reloaded.project(labs).map(|project| project.name.as_str()),
            Some("labs")
        );
        Ok(())
    }
}
