//! The typed tree of the desktop configuration file, and its conversion to and from the aggregate.
//!
//! The tree is the fractal view of the configuration (ADR 0011). A project node is a name plus
//! slots, and the document root is the one node whose name is implied, because the terminal
//! synthesizes the root project `Projects`. A slot hosts either a launcher or a nested project
//! node. Ids are not stored: projects and launchers get fresh ids at load, and the file
//! identifies content by nesting and position alone. [`super::kdl_format`] maps the tree to KDL
//! text.

use std::collections::BTreeMap;

use log::warn;
use massive_applications::McpConfiguration;

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
    pub slots: PersistedSlots,
}

/// The assigned slots of a project node, keyed by position in matrix order. Two slots never share a
/// position, and the map cannot hold two. Only assigned slots are stored.
pub type PersistedSlots = BTreeMap<SlotPosition, PersistedSlot>;

/// What one assigned slot of a project node hosts. The project variant is the fractal unit of the
/// document. The document root is the
/// one project without a slot, so it carries no name and the terminal synthesizes
/// [`ROOT_PROJECT_NAME`] for it.
#[derive(Debug, Clone, PartialEq)]
pub enum PersistedSlot {
    Launcher {
        name: String,
        params: Params,
        /// The launcher's Full Screen Mode, shared by its primary instances (ADR 0014).
        full_screen_mode: FullScreenMode,
    },
    Project {
        name: String,
        /// Present exactly when the project is an MCP root (ADR 0036).
        mcp: Option<McpConfiguration>,
        slots: PersistedSlots,
    },
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

    /// How many levels of projects nest below the root project: 0 without projects, 1 for projects
    /// in the root's slots, and one more for each level below. The load checks it against the
    /// limit without building the aggregate.
    pub fn project_depth(&self) -> usize {
        slots_depth(&self.slots)
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
    slots: &PersistedSlots,
    mcp_root_above: bool,
    projects: &mut Vec<Project>,
) -> Vec<Slot> {
    let mut parsed: Vec<Slot> = Vec::new();
    for (&position, slot) in slots {
        match slot {
            PersistedSlot::Launcher {
                name,
                params,
                full_screen_mode,
            } => parsed.push(Slot::launcher(
                position,
                Launcher::new(name.clone(), params.clone())
                    .with_full_screen_mode(*full_screen_mode),
            )),
            PersistedSlot::Project { name, mcp, slots } => {
                let mut mcp = mcp.clone();
                if mcp_root_above && mcp.is_some() {
                    warn!(
                        "Project '{name}' lies inside another MCP root, ignoring its MCP settings"
                    );
                    mcp = None;
                }
                let nested = persisted_slots(slots, mcp_root_above || mcp.is_some(), projects);
                let mut project = Project::new(name.clone(), nested);
                project.mcp = mcp;
                let id = project.id;
                projects.push(project);
                parsed.push(Slot::project(position, id));
            }
        }
    }
    parsed
}

/// How many levels of projects the slots bring along, recursing into each nested project.
fn slots_depth(slots: &PersistedSlots) -> usize {
    slots
        .values()
        .map(|slot| match slot {
            PersistedSlot::Launcher { .. } => 0,
            PersistedSlot::Project { slots, .. } => 1 + slots_depth(slots),
        })
        .max()
        .unwrap_or(0)
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
fn project_slots(configuration: &RuntimeConfiguration, project: ProjectId) -> PersistedSlots {
    configuration[project]
        .slots()
        .iter()
        .map(|slot| {
            let persisted = match &slot.content {
                SlotPayload::Launcher(launcher) => PersistedSlot::Launcher {
                    name: launcher.name.clone(),
                    params: launcher.params.clone(),
                    full_screen_mode: launcher.full_screen_mode,
                },
                SlotPayload::Project(child) => PersistedSlot::Project {
                    name: configuration[*child].name.clone(),
                    mcp: configuration[*child].mcp.clone(),
                    slots: project_slots(configuration, *child),
                },
            };
            (slot.position, persisted)
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
