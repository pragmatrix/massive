use std::collections::BTreeSet;
use std::str::FromStr;

use anyhow::{Error, bail};
use serde::{Deserialize, Serialize};

/// What the user configured on a project that is an MCP root (ADR 0036). A project without one is
/// no MCP root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpConfiguration {
    pub access_scopes: AccessScopes,
}

/// The access scopes an MCP root grants. An empty set grants nothing, so agents see no tools. The
/// set is ordered, so the project header and the configuration file list the scopes in the same
/// order every time.
pub type AccessScopes = BTreeSet<AccessScope>;

/// A named permission an MCP root grants to the agents that connect to it (ADR 0036). Every tool
/// declares exactly one access scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessScope {
    /// Describing the covered subtree.
    Observe,
    /// Every configuration command inside the MCP root's subtree.
    Organize,
    /// Application tools that only read an instance's content.
    Read,
    /// Starting and stopping instances, and application tools that change something.
    Act,
}

impl AccessScope {
    pub const ALL: [Self; 4] = [Self::Observe, Self::Organize, Self::Read, Self::Act];

    /// The name the CLI, the configuration file and the project header use.
    pub fn name(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Organize => "organize",
            Self::Read => "read",
            Self::Act => "act",
        }
    }
}

impl FromStr for AccessScope {
    type Err = Error;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        let Some(scope) = Self::ALL.into_iter().find(|scope| scope.name() == name) else {
            bail!("unknown access scope '{name}', expected one of observe, organize, read, act");
        };
        Ok(scope)
    }
}
