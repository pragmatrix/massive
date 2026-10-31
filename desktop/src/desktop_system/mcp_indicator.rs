//! The MCP indicators show the user where agents can act through MCP, and what they may do there
//! (ADR 0036). The MCP activation indicator gives every header, launcher, and instance title bar an
//! MCP root covers the colors of [`colors::mcp`]. The MCP access indicator names the access scopes
//! in the MCP root's header.
//!
//! A presenter takes its indicators when it is created. A configuration change that can change
//! which MCP root covers existing content names a project in its change surface, and the
//! transaction's effects refresh the indicators of that project's whole subtree.

use massive_applications::McpConfiguration;
use massive_geometry::Color;

use crate::desktop_system::{DesktopEffect, DesktopSystem, DesktopTarget, Effects};
use crate::instance_presenter::InstanceKind;
use crate::policy::colors;
use crate::projects::{ProjectId, SlotContent};

impl DesktopSystem {
    /// Refreshes the indicators of `project` and everything below it. A project the transaction
    /// removed has nothing left to refresh.
    pub(super) fn refresh_mcp_indicators_effect(&mut self, project: ProjectId) -> Effects {
        if self.aggregates.configuration.project(project).is_none() {
            return Effects::None;
        }
        let mcp_activation = self.mcp_activation(project);
        self.refresh_content_indicators(SlotContent::Project(project), mcp_activation);
        // Configuring MCP changes the project's header text, so the header needs a new measure.
        [DesktopEffect::Measure(DesktopTarget::header(project))].into()
    }

    /// Whether an MCP root covers `project`, and with it everything in its matrix.
    pub(super) fn mcp_activation(&self, project: ProjectId) -> McpActivation {
        match self.aggregates.configuration.mcp_root_of(project) {
            Some(_) => McpActivation::Active,
            None => McpActivation::Inactive,
        }
    }

    pub(super) fn project_header_text(&self, project: ProjectId) -> String {
        let project = &self.aggregates.configuration[project];
        project_header_text(&project.name, project.mcp.as_ref())
    }

    /// An MCP root covers its whole subtree, so the MCP activation found at `content` holds for
    /// everything below it. A project that is itself an MCP root activates MCP for its subtree, even
    /// when nothing above it does.
    fn refresh_content_indicators(&mut self, content: SlotContent, mcp_activation: McpActivation) {
        match content {
            SlotContent::Launcher(launcher) => {
                self.aggregates
                    .launchers
                    .get_mut(&launcher)
                    .expect("a configured launcher has a presenter")
                    .set_mcp_activation(mcp_activation);
                let instances: Vec<_> = self
                    .aggregates
                    .hierarchy
                    .launcher_instances(launcher)
                    .collect();
                for instance in instances {
                    self.aggregates
                        .instances
                        .get_mut(&instance)
                        .expect("a running instance has a presenter")
                        .set_mcp_activation(mcp_activation);
                }
            }
            SlotContent::Project(project) => {
                let mcp_activation = match self.aggregates.configuration[project].mcp {
                    Some(_) => McpActivation::Active,
                    None => mcp_activation,
                };
                let text = self.project_header_text(project);
                self.aggregates
                    .projects
                    .get_mut(&project)
                    .expect("a configured project has a presenter")
                    .set_header(&text, mcp_activation);
                let slots: Vec<_> = self
                    .aggregates
                    .configuration
                    .slots_ordered(project)
                    .map(|(_, content)| content)
                    .collect();
                for content in slots {
                    self.refresh_content_indicators(content, mcp_activation);
                }
            }
        }
    }
}

/// Whether MCP is active for a project, a launcher, or an instance, which is the case exactly when an
/// MCP root covers it. Each presenter picks the colors of its MCP activation indicator from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpActivation {
    Inactive,
    Active,
}

impl McpActivation {
    pub fn project_header_background(self) -> Color {
        match self {
            Self::Inactive => colors::project::HEADER_BACKGROUND,
            Self::Active => colors::mcp::PROJECT_HEADER_BACKGROUND,
        }
    }

    pub fn launcher_background(self) -> Color {
        match self {
            Self::Inactive => colors::launcher::BACKGROUND,
            Self::Active => colors::mcp::LAUNCHER_BACKGROUND,
        }
    }

    /// Assistant instances keep a color of their own, inside an MCP root too.
    pub fn instance_title_bar_background(self, kind: InstanceKind) -> Color {
        match (self, kind) {
            (Self::Inactive, InstanceKind::Primary) => {
                colors::instance::PRIMARY_TITLE_BAR_BACKGROUND
            }
            (Self::Inactive, InstanceKind::Assistant) => {
                colors::instance::ASSISTANT_TITLE_BAR_BACKGROUND
            }
            (Self::Active, InstanceKind::Primary) => colors::mcp::PRIMARY_TITLE_BAR_BACKGROUND,
            (Self::Active, InstanceKind::Assistant) => colors::mcp::ASSISTANT_TITLE_BAR_BACKGROUND,
        }
    }
}

/// The text of a project's header: its name, followed by the MCP access indicator when the project
/// is an MCP root, as in `home · MCP observe organize`.
fn project_header_text(name: &str, mcp: Option<&McpConfiguration>) -> String {
    let Some(mcp) = mcp else {
        return name.to_string();
    };
    let mut text = format!("{name} · MCP");
    for scope in &mcp.access_scopes {
        text.push(' ');
        text.push_str(scope.name());
    }
    text
}
