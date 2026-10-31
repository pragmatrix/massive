//! The desktop configuration document view: the JSON representation, persisted
//! atomically at the end of a transaction.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

use super::persisted::PersistedConfiguration;
use super::write_atomic;
use crate::policy::constants;
use crate::policy::when::{self, NestingParams, NestingProjects};
use crate::projects::{ProjectId, RuntimeConfiguration, SlotContent, params_depth};

/// The document view of the desktop configuration: the path the JSON document,
/// derived from the aggregate, is written to atomically.
///
/// The document is derived from the aggregate, so a change is applied to the
/// aggregate first and the document is rebuilt from it when persisted — there is no
/// separate edit path to keep in sync. The file is written synchronously, once
/// per transaction, so a transaction's several changes persist as a single
/// write.
#[derive(Debug)]
pub struct ConfigurationPersistence {
    /// Where the configuration lives. Created at load time: the default
    /// configuration is written when no file exists yet, so every later change has
    /// a file to be persisted to.
    path: PathBuf,
}

impl ConfigurationPersistence {
    /// Creates the file view for an existing or soon-to-exist configuration
    /// file. Infallible: reading and parsing happen elsewhere.
    pub fn new(path: &Path) -> Self {
        Self { path: path.into() }
    }

    /// Writes the document derived from the aggregate.
    ///
    /// Called only for a transaction that changed the configuration, so its several changes
    /// persist as one file write. A write failure is logged; the next change writes the whole
    /// document again.
    pub fn persist(&self, configuration: &RuntimeConfiguration) {
        let text = match serde_json::to_string_pretty(&PersistedConfiguration::from_configuration(
            configuration,
        )) {
            Ok(text) => text,
            Err(error) => {
                log::warn!("Failed to serialize the configuration: {error}");
                return;
            }
        };
        match write_atomic(&self.path, &text) {
            Ok(()) => log::info!("Configuration persisted to {}", self.path.display()),
            Err(error) => log::warn!(
                "Failed to persist configuration to {}: {error:#}",
                self.path.display()
            ),
        }
    }
}

/// Parses the aggregate from configuration text; parse errors fail. Deriving
/// the aggregate cannot: the document is self-contained.
///
/// A configuration that nests deeper than the desktop allows fails to load, with an error that
/// names the depth and the limit. Every change keeps within the limits, so only a hand edit gets
/// there, and the user fixes the file.
pub fn parse_configuration(path: &Path, text: &str) -> Result<RuntimeConfiguration> {
    let document: PersistedConfiguration =
        serde_json::from_str(text).with_context(|| format!("parsing {}", path.display()))?;
    let configuration = document.into_configuration();

    // The root project counts as one level of its own nesting height.
    let deepest_project = configuration.nesting_height(SlotContent::Project(ProjectId::ROOT)) - 1;
    ensure!(
        when::nesting_projects(Some(deepest_project)) == NestingProjects::Allow,
        "{} nests projects {deepest_project} levels deep, at most {} are allowed",
        path.display(),
        constants::configuration::MAX_PROJECT_DEPTH
    );
    let deepest_params = configuration
        .launchers()
        .map(|launcher| params_depth(&launcher.params))
        .max()
        .unwrap_or(0);
    ensure!(
        when::nesting_params(deepest_params) == NestingParams::Allow,
        "{} nests launcher params {deepest_params} levels deep, at most {} are allowed",
        path.display(),
        constants::configuration::MAX_PARAMS_DEPTH
    );
    Ok(configuration)
}

/// Writes the built-in default configuration to the file, for callers that decide
/// a missing configuration file means "start fresh".
pub fn write_default_config(path: &Path) -> Result<()> {
    let document = PersistedConfiguration::default_document();
    let text =
        serde_json::to_string_pretty(&document).context("serializing the default configuration")?;
    write_atomic(path, &text)
        .with_context(|| format!("writing the default configuration to {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::projects::{ProjectId, SlotAssignment, SlotPosition};

    /// A configuration whose projects and params nest as deep as the limits allow saves and loads
    /// again, so no configuration the desktop accepts keeps it from starting. A change to the
    /// saved format that adds nesting levels fails here first.
    #[test]
    fn a_configuration_at_the_nesting_limits_saves_and_loads() -> Result<()> {
        use crate::policy::constants::configuration::{MAX_PARAMS_DEPTH, MAX_PROJECT_DEPTH};
        use crate::projects::{LaunchProfile, LaunchProfileId, LauncherMode, params_depth};

        let dir = tempfile::tempdir()?;
        let path = dir.path().join("desktop.json");
        let mut configuration = parse_configuration(path.as_path(), r#"{ "slots": [] }"#)?;
        let first = SlotPosition { column: 0, row: 0 };
        let mut deepest = ProjectId::ROOT;
        for _ in 0..MAX_PROJECT_DEPTH {
            let nested = ProjectId::new();
            configuration.assign_slot(
                Some(deepest),
                first,
                SlotAssignment::Project {
                    id: nested,
                    name: "nested".into(),
                },
            );
            deepest = nested;
        }
        let mut value = serde_json::json!(1);
        for _ in 1..MAX_PARAMS_DEPTH {
            value = serde_json::json!({ "a": value });
        }
        let params = serde_json::Map::from_iter([("a".to_string(), value)]);
        assert_eq!(params_depth(&params), MAX_PARAMS_DEPTH);
        configuration.assign_slot(
            Some(deepest),
            first,
            SlotAssignment::Launcher {
                id: LaunchProfileId::new(),
                profile: LaunchProfile {
                    name: "shell".into(),
                    mode: LauncherMode::Visor,
                    params,
                    full_screen_mode: Default::default(),
                },
            },
        );
        assert_eq!(configuration.project_depth(deepest), MAX_PROJECT_DEPTH);

        ConfigurationPersistence::new(path.as_path()).persist(&configuration);
        parse_configuration(path.as_path(), &fs::read_to_string(&path)?)?;
        Ok(())
    }

    /// A file that nests deeper than either limit fails to load, and the error names the depth, so
    /// the user can find and fix it. Only a hand edit gets a file there.
    #[test]
    fn a_configuration_past_a_nesting_limit_fails_to_load() {
        use crate::policy::constants::configuration::{MAX_PARAMS_DEPTH, MAX_PROJECT_DEPTH};

        let path = Path::new("desktop.json");
        let launcher = |params: &str| {
            format!(
                r#"{{ "at": [0, 0], "launcher": {{ "name": "shell", "mode": "visor", "params": {params} }} }}"#
            )
        };
        let nested = |depth: usize, innermost: String| {
            (0..depth).fold(innermost, |inner, _| {
                format!(r#"{{ "at": [0, 0], "project": {{ "name": "p", "slots": [ {inner} ] }} }}"#)
            })
        };
        let document = |slot: String| format!(r#"{{ "slots": [ {slot} ] }}"#);
        let params = |depth: usize| format!("{}1{}", r#"{"a":"#.repeat(depth), "}".repeat(depth));

        let at_limits = document(nested(
            MAX_PROJECT_DEPTH,
            launcher(&params(MAX_PARAMS_DEPTH)),
        ));
        assert!(parse_configuration(path, &at_limits).is_ok());

        let too_deep = document(nested(MAX_PROJECT_DEPTH + 1, launcher("{}")));
        let error = parse_configuration(path, &too_deep).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("{} levels deep", MAX_PROJECT_DEPTH + 1)),
            "{error}"
        );
        let deep_params = document(launcher(&params(MAX_PARAMS_DEPTH + 1)));
        assert!(parse_configuration(path, &deep_params).is_err());
    }

    /// Persisting writes the aggregate as it is: the assigned project reaches the file.
    #[test]
    fn a_persisted_change_reaches_the_file() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("desktop.json");
        let mut configuration = parse_configuration(
            path.as_path(),
            r#"{ "slots": [ { "at": [0, 0], "launcher": { "name": "shell", "mode": "visor" } } ] }"#,
        )?;
        let document = ConfigurationPersistence::new(path.as_path());
        let root = ProjectId::ROOT;

        configuration.assign_slot(
            Some(root),
            SlotPosition { column: 1, row: 0 },
            SlotAssignment::Project {
                id: ProjectId::new(),
                name: "extra".into(),
            },
        );
        document.persist(&configuration);

        let text = fs::read_to_string(&path)?;
        assert!(
            text.contains("\"name\": \"extra\""),
            "unexpected document: {text}"
        );
        Ok(())
    }
}
