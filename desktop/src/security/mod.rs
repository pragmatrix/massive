//! Every security-relevant check of the desktop (ADR 0036).
//!
//! A check guards what a process outside the desktop may do through a channel the desktop controls.
//! Each check is a pure function of the facts its caller gathers, and it returns an allowance or a
//! refusal whose variant names the reason. The caller carries out the answer. Checks are not
//! desktop policy (ADR 0029), because they enforce a boundary that holds regardless of taste.
//! `docs/security.md` lists every check, the trust model, and the known ways around the checks.

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
