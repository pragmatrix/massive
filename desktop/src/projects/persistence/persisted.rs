//! The typed tree of the desktop configuration file, and its conversion to and from the aggregate.
//!
//! The tree is the fractal view of the configuration (ADR 0011). A project node is a name plus
//! slots, and the document root is the one node whose name is implied, because the terminal
//! synthesizes the root project `Projects`. A slot hosts either a launcher or a nested project
//! node. Ids are not stored: projects and launchers get fresh ids at load, and the file
//! identifies content by nesting and position alone. [`super::kdl_format`] maps the tree to KDL
//! text.

use log::warn;
use massive_applications::{AccessScopes, McpConfiguration};

use crate::policy::when::{self, LoadingWithoutLauncher};
use crate::projects::{
    DEFAULT_NEW_LAUNCHER_NAME, FullScreenMode, LaunchProfile, LaunchProfileId, Launcher, Params,
    Project, ProjectId, ROOT_PROJECT_NAME, RuntimeConfiguration, Slot, SlotAssignment, SlotPayload,
    SlotPosition,
};

/// The document form of [`crate::projects::DesktopConfiguration`]: the startup
/// target path and the root project's slots, with every nested project hanging
/// off the slot that hosts it.
#[derive(Debug, Clone, PartialEq)]
pub struct PersistedConfiguration {
    pub startup: Option<String>,
    pub slots: Vec<PersistedSlot>,
}

/// One assigned slot of a project node: its position plus exactly one content
/// variant. Only assigned slots are stored.
#[derive(Debug, Clone, PartialEq)]
pub struct PersistedSlot {
    /// The position as `(column, row)`.
    pub at: (u32, u32),
    pub content: PersistedSlotContent,
}

/// What a slot hosts: a launcher or a nested project, never both.
#[derive(Debug, Clone, PartialEq)]
pub enum PersistedSlotContent {
    Launcher(PersistedLauncher),
    Project(PersistedProject),
}

/// A launcher's spawnable profile.
#[derive(Debug, Clone, PartialEq)]
pub struct PersistedLauncher {
    pub name: String,
    pub params: Params,
    /// The launcher's Full Screen Mode, shared by its primary instances (ADR 0014).
    pub full_screen_mode: FullScreenMode,
}

/// A project node: the fractal unit of the document. The name is required — the
/// document root is the one node without a `project` wrapper, so it carries no
/// name and the terminal synthesizes [`ROOT_PROJECT_NAME`] for it.
#[derive(Debug, Clone, PartialEq)]
pub struct PersistedProject {
    pub name: String,
    pub mcp: Option<PersistedMcp>,
    pub slots: Vec<PersistedSlot>,
}

/// The MCP settings of a project node, present exactly when the project is an MCP root.
#[derive(Debug, Clone, PartialEq)]
pub struct PersistedMcp {
    pub access_scopes: AccessScopes,
}

impl PersistedConfiguration {
    /// Derives the aggregate from the document: a depth-first walk that gives
    /// every project node a fresh id and links children through their slots,
    /// then hands the flat project list to the configuration constructor, which
    /// resolves the startup path and synthesizes the root.
    ///
    /// A configuration that defines no launcher is given one in the root project
    /// (ADR 0012): no spawn parameters, and the first free
    /// position of the root matrix. The launcher is part of the aggregate, so
    /// the first persist writes it and a re-load finds it instead of adding a
    /// second one.
    pub fn into_configuration(self) -> RuntimeConfiguration {
        let mut projects = Vec::new();
        let root_slots = persisted_slots(&self.slots, false, &mut projects);

        let mut root = Project::new(ROOT_PROJECT_NAME.into(), root_slots);
        root.id = ProjectId::ROOT;
        projects.push(root);

        let mut configuration = RuntimeConfiguration::new(projects, self.startup.as_deref());
        ensure_launcher(&mut configuration);
        configuration
    }

    /// Builds the document from the aggregate: a walk from the root through the
    /// slot links, rebuilding the tree. The aggregate keeps every project's
    /// slots in position order, so the output is deterministic. The walk reads
    /// only what the id-link invariant guarantees, so it cannot fail.
    pub fn from_configuration(configuration: &RuntimeConfiguration) -> Self {
        Self {
            startup: configuration.startup_path().map(str::to_owned),
            slots: project_slots(configuration, ProjectId::ROOT),
        }
    }
}
/// Parses a project node's slots, pushing every nested project into `projects`
/// before the project hosting it — the order the old parse produced, so
/// document-order lookups keep their semantics.
///
/// `mcp_root_above` tells whether an MCP root lies above the node. A hand-edited file
/// may nest MCP roots, which the desktop never allows (ADR 0036), so the
/// outermost root stays and every root below it loses its `mcp` object.
fn persisted_slots(
    slots: &[PersistedSlot],
    mcp_root_above: bool,
    projects: &mut Vec<Project>,
) -> Vec<Slot> {
    let mut parsed: Vec<Slot> = Vec::new();
    for slot in slots {
        let position = SlotPosition::from(slot.at);
        match &slot.content {
            PersistedSlotContent::Launcher(launcher) => parsed.push(Slot::launcher(
                position,
                Launcher::new(launcher.name.clone(), launcher.params.clone())
                    .with_full_screen_mode(launcher.full_screen_mode),
            )),
            PersistedSlotContent::Project(persisted) => {
                let mut mcp = persisted.mcp.as_ref().map(|mcp| McpConfiguration {
                    access_scopes: mcp.access_scopes.clone(),
                });
                if mcp_root_above && mcp.is_some() {
                    warn!(
                        "Project '{}' lies inside another MCP root, ignoring its MCP settings",
                        persisted.name
                    );
                    mcp = None;
                }
                let nested =
                    persisted_slots(&persisted.slots, mcp_root_above || mcp.is_some(), projects);
                let mut project = Project::new(persisted.name.clone(), nested);
                project.mcp = mcp;
                let id = project.id;
                projects.push(project);
                parsed.push(Slot::project(position, id));
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

    match when::loading_without_launcher() {
        LoadingWithoutLauncher::AddDefaultLauncher => {
            // The new row below the root's slots displaces nothing.
            let position = SlotPosition {
                column: 0,
                row: configuration
                    .slots_ordered(ProjectId::ROOT)
                    .map(|(position, _)| position.row)
                    .max()
                    .map_or(0, |row| row + 1),
            };
            let profile = LaunchProfile {
                name: DEFAULT_NEW_LAUNCHER_NAME.into(),
                params: Params::new(),
                full_screen_mode: FullScreenMode::Regular,
            };
            let assignment = SlotAssignment::Launcher {
                id: LaunchProfileId::new(),
                profile,
            };
            configuration.assign_slot(Some(ProjectId::ROOT), position, assignment);
        }
    }
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
            at: (slot.position.column, slot.position.row),
            content: match &slot.content {
                SlotPayload::Launcher(launcher) => {
                    PersistedSlotContent::Launcher(PersistedLauncher {
                        name: launcher.name.clone(),
                        params: launcher.params.clone(),
                        full_screen_mode: launcher.full_screen_mode,
                    })
                }
                SlotPayload::Project(child) => PersistedSlotContent::Project(PersistedProject {
                    name: configuration[*child].name.clone(),
                    mcp: configuration[*child].mcp.as_ref().map(|mcp| PersistedMcp {
                        access_scopes: mcp.access_scopes.clone(),
                    }),
                    slots: project_slots(configuration, *child),
                }),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use std::path::Path;

    use super::*;
    use crate::projects::persistence::kdl_format;
    use massive_applications::AccessScope;
    use serde_json::Value;

    #[test]
    fn a_document_writes_the_fractal_shape() {
        let document = PersistedConfiguration {
            startup: Some("/labs/shell".into()),
            slots: vec![
                PersistedSlot {
                    at: (0, 0),
                    content: PersistedSlotContent::Launcher(PersistedLauncher {
                        name: "shell".into(),
                        params: Params::new(),
                        full_screen_mode: FullScreenMode::Regular,
                    }),
                },
                PersistedSlot {
                    at: (1, 0),
                    content: PersistedSlotContent::Project(PersistedProject {
                        name: "labs".into(),
                        mcp: None,
                        slots: vec![PersistedSlot {
                            at: (0, 0),
                            content: PersistedSlotContent::Launcher(PersistedLauncher {
                                name: "build".into(),
                                params: [("command".into(), Value::String("just".into()))]
                                    .into_iter()
                                    .collect(),
                                full_screen_mode: FullScreenMode::Regular,
                            }),
                        }],
                    }),
                },
            ],
        };

        assert_eq!(
            kdl_format::write_document(&document).to_string(),
            "startup \"/labs/shell\"\n\
             launcher shell at=\"0,0\"\n\
             project labs at=\"1,0\" {\n    launcher build at=\"0,0\" {\n        command just\n    }\n}\n"
        );
    }

    /// The document is derived from the aggregate and back without loss: the
    /// nesting, positions, launcher profiles, MCP settings, and the startup path survive.
    #[test]
    fn the_aggregate_round_trips_through_the_document() -> Result<()> {
        let text = r#"
            startup "/labs/build"
            launcher shell at="0,0"
            project labs at="1,0" {
                mcp act organize
                launcher build at="0,0" {
                    command just
                }
            }
        "#;
        let configuration = read(text)?.into_configuration();

        let written =
            kdl_format::write_document(&PersistedConfiguration::from_configuration(&configuration))
                .to_string();
        let reloaded = read(&written)?.into_configuration();

        assert_eq!(reloaded.startup_path(), configuration.startup_path());
        assert_eq!(reloaded.launcher_count(), configuration.launcher_count());
        let root = ProjectId::ROOT;
        let labs = reloaded
            .child_project_at(root, SlotPosition { column: 1, row: 0 })
            .expect("labs is a root slot");
        assert_eq!(
            reloaded.project(labs).map(|project| project.name.as_str()),
            Some("labs")
        );
        assert_eq!(
            reloaded[labs].mcp,
            Some(McpConfiguration {
                access_scopes: [AccessScope::Organize, AccessScope::Act].into()
            })
        );
        Ok(())
    }

    /// A hand-edited file can nest MCP roots, which the desktop never allows (ADR 0036). The load
    /// keeps the outermost one.
    #[test]
    fn a_nested_mcp_root_loses_its_mcp_settings_at_load() -> Result<()> {
        let text = r#"
            project outer at="0,0" {
                mcp act
                project inner at="0,0" {
                    mcp
                }
            }
        "#;
        let configuration = read(text)?.into_configuration();

        let outer = configuration
            .child_project_at(ProjectId::ROOT, SlotPosition { column: 0, row: 0 })
            .expect("outer is a root slot");
        let inner = configuration
            .child_project_at(outer, SlotPosition { column: 0, row: 0 })
            .expect("inner is a slot of outer");
        assert_eq!(
            configuration[outer].mcp,
            Some(McpConfiguration {
                access_scopes: [AccessScope::Act].into()
            })
        );
        assert_eq!(configuration[inner].mcp, None);
        Ok(())
    }

    fn read(text: &str) -> Result<PersistedConfiguration> {
        kdl_format::read_configuration(Path::new("desktop.kdl"), text)
    }
}
