//! Every security-relevant check of the desktop (ADR 0036).
//!
//! A check guards what a process outside the desktop may do through a channel the desktop controls.
//! Each check is a pure function of the facts its caller gathers, and it returns an allowance or a
//! refusal whose variant names the reason. The caller carries out the answer. Checks are not
//! desktop policy (ADR 0029), because they enforce a boundary that holds regardless of taste.
//! `docs/security.md` lists every check, the trust model, and the known ways around the checks.

use massive_applications::{AccessScope, AccessScopes};

use crate::projects::{MatrixPoint, MatrixRect, ProjectId};

/// Whether a configuration request may configure MCP on any project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfiguringMcp {
    Allow,
    /// An agent inside an MCP root must not enable or disable MCP, or widen or narrow any MCP
    /// root's access scopes, its own or another's.
    RefuseFromCoveredInstance,
}

/// `issued_by_covered_instance` tells whether the request comes from an instance that any MCP root
/// covers. The desktop knows the issuing instance by the file descriptor it inherited, so a process
/// cannot claim to be elsewhere.
#[must_use]
pub fn configuring_mcp(issued_by_covered_instance: bool) -> ConfiguringMcp {
    if issued_by_covered_instance {
        return ConfiguringMcp::RefuseFromCoveredInstance;
    }
    ConfiguringMcp::Allow
}

/// Whether a project may be an MCP root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakingMcpRoot {
    Allow,
    /// The root project would cover every instance on the desktop, so no process could disable it
    /// again from the CLI.
    RefuseRootProject,
    /// MCP roots never nest, so a connection's MCP root is determined without a choice.
    RefuseNested,
}

/// `mcp_root_above` and `mcp_root_below` tell whether another MCP root lies above or below the
/// project. The project itself does not count, so changing the access scopes of an existing MCP
/// root is allowed.
#[must_use]
pub fn making_mcp_root(
    is_root_project: bool,
    mcp_root_above: bool,
    mcp_root_below: bool,
) -> MakingMcpRoot {
    if is_root_project {
        return MakingMcpRoot::RefuseRootProject;
    }
    if mcp_root_above || mcp_root_below {
        return MakingMcpRoot::RefuseNested;
    }
    MakingMcpRoot::Allow
}

/// What the desktop answers a tool client that connects to a target (ADR 0036).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectingToTarget {
    Bind(ProjectId),
    /// A target that does not exist is declined like one outside every MCP root, so a tool client
    /// learns nothing about what lies outside every MCP root.
    Decline,
}

/// `covering_mcp_root` is the MCP root that covers the target, or `None` when the target does not
/// exist or no MCP root covers it.
#[must_use]
pub fn connecting_to_target(covering_mcp_root: Option<ProjectId>) -> ConnectingToTarget {
    match covering_mcp_root {
        Some(root) => ConnectingToTarget::Bind(root),
        None => ConnectingToTarget::Decline,
    }
}

/// The reason every declined tool connection gets, whatever the target was.
pub const DECLINED_TARGET: &str = "this process is not inside an MCP root";

/// Whether a tool connection may call a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallingTool {
    Allow,
    /// The MCP root does not grant the tool's access scope.
    RefuseScopeNotGranted,
}

#[must_use]
pub fn calling_tool(granted: &AccessScopes, needed: AccessScope) -> CallingTool {
    if granted.contains(&needed) {
        return CallingTool::Allow;
    }
    CallingTool::RefuseScopeNotGranted
}

/// Whether a tool call may name a project, a launcher or an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamingTarget {
    Allow,
    /// The target does not exist, or the connection's MCP root does not cover it. The refusal does
    /// not say which, so an agent learns nothing about what lies outside its MCP root.
    RefuseUnknown,
}

/// Refuses a target that the connection's MCP root `root` does not cover, with the same answer as
/// for a target that does not exist, so nothing leaks from outside the root. `covering_mcp_root`
/// is the MCP root that covers the target, or `None` when the target does not exist or no MCP root
/// covers it.
#[must_use]
pub fn naming_target(covering_mcp_root: Option<ProjectId>, root: ProjectId) -> NamingTarget {
    if covering_mcp_root == Some(root) {
        return NamingTarget::Allow;
    }
    NamingTarget::RefuseUnknown
}

/// Whether a tool call may name a slot of a covered project's matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamingSlot {
    Allow,
    /// The desktop lays out every column and row up to the slot, so a far slot would make it lay
    /// out a vast empty matrix.
    RefuseOutsideMatrix,
}

/// `matrix` is the rectangle of the project's matrix. A slot one column right of it or one row
/// below it is allowed, so an agent can grow the matrix.
#[must_use]
pub fn naming_slot(slot: MatrixPoint, matrix: MatrixRect) -> NamingSlot {
    if slot.x <= matrix.max.x && slot.y <= matrix.max.y {
        return NamingSlot::Allow;
    }
    NamingSlot::RefuseOutsideMatrix
}

/// Whether a tool call may remove slot content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovingContent {
    Allow,
    /// Removing the content stops the instances that run below it, and stopping instances needs
    /// `act`.
    RefuseWithoutAct,
}

#[must_use]
pub fn removing_content(instances_run_below: bool, granted: &AccessScopes) -> RemovingContent {
    if instances_run_below && !granted.contains(&AccessScope::Act) {
        return RemovingContent::RefuseWithoutAct;
    }
    RemovingContent::Allow
}

/// Whether a tool call may create a launcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatingLauncher {
    Allow,
    /// A launcher's parameters decide what its instances run, such as a terminal's command. The
    /// user runs it by starting an instance, so setting them needs `act`, like running it does.
    RefuseParamsWithoutAct,
}

#[must_use]
pub fn creating_launcher(sets_params: bool, granted: &AccessScopes) -> CreatingLauncher {
    if sets_params && !granted.contains(&AccessScope::Act) {
        return CreatingLauncher::RefuseParamsWithoutAct;
    }
    CreatingLauncher::Allow
}
