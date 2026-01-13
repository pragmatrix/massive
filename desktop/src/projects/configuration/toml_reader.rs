use std::collections::HashMap;

use anyhow::Result;
use serde::Deserialize;
use toml::Value;

use super::types::{
    GroupContents, LaunchGroup, LaunchProfile, LayoutDirection, Parameter, Parameters, ScopedTag,
};

/// Intermediate representation for deserializing TOML configuration files.
#[derive(Debug, Deserialize)]
pub struct ConfigFile {
    /// The startup launch profile.
    ///
    /// If None is defined, a launch profile without a name is created and started.
    #[serde(default)]
    pub startup: Option<String>,
    #[serde(default)]
    pub layout: Option<LayoutSection>,
    #[serde(flatten)]
    pub launch_profiles: HashMap<String, LaunchProfileSection>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LayoutSection {
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub order: HashMap<String, Vec<String>>,
}

pub type LaunchProfileSection = HashMap<String, Value>;

impl ConfigFile {
    /// Convert the intermediate TOML representation into a launch group, which is itself
    /// an ApplicationGroup.
    pub fn into_launch_group(self, name: String) -> Result<LaunchGroup> {
        let layout = self.layout.unwrap_or_default();
        let group_tags = &layout.groups;

        // Build ApplicationRefs from each application section
        let app_refs: Vec<LaunchProfile> = self
            .launch_profiles
            .into_iter()
            .map(|(name, section)| build_launch_profile(name, section, group_tags))
            .collect::<Result<Vec<_>>>()?;

        // Build the cross-product hierarchy
        let app_refs: Vec<_> = app_refs.iter().collect();
        let groups = build_group_hierarchy(&app_refs, group_tags, &layout.order, 0)?;

        Ok(LaunchGroup {
            name,
            tag: ScopedTag::new("", ""),
            layout: LayoutDirection::Horizontal,
            content: GroupContents::Groups(groups),
        })
    }
}

fn build_launch_profile(
    name: String,
    section: LaunchProfileSection,
    group_tags: &[String],
) -> Result<LaunchProfile> {
    let mut tags = Vec::new();
    let mut params = Vec::new();

    for (key, value) in section {
        let value_str = toml_value_to_string(&value)?;

        if group_tags.contains(&key) {
            tags.push(ScopedTag::new(key, value_str));
        } else {
            params.push(Parameter::new(key, value_str));
        }
    }

    Ok(LaunchProfile {
        name,
        params: Parameters(params),
        tags,
    })
}

fn toml_value_to_string(value: &Value) -> Result<String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Integer(i) => Ok(i.to_string()),
        Value::Float(f) => Ok(f.to_string()),
        Value::Boolean(b) => Ok(b.to_string()),
        _ => anyhow::bail!("Unsupported TOML value type: {:?}", value),
    }
}

/// Build a cross-product hierarchy of groups at the given depth level.
fn build_group_hierarchy(
    apps: &[&LaunchProfile],
    group_tags: &[String],
    order: &HashMap<String, Vec<String>>,
    depth: usize,
) -> Result<Vec<LaunchGroup>> {
    if depth >= group_tags.len() {
        return Ok(Vec::new());
    }

    let current_tag_name = &group_tags[depth];

    // Collect all unique values for this tag from applications
    let mut found_values: HashMap<String, Vec<&LaunchProfile>> = HashMap::new();
    for app in apps {
        if let Some(tag) = app.tags.iter().find(|t| &t.scope == current_tag_name) {
            found_values.entry(tag.tag.clone()).or_default().push(app);
        }
    }

    let ordered_values = {
        match order.get(current_tag_name) {
            Some(v) => v.clone(),
            None => {
                // No ordered specification, take all the values we have and sort it alphabetically.
                let mut keys: Vec<_> = found_values.keys().cloned().collect();
                keys.sort();
                keys
            }
        }
    };

    let mut groups = Vec::new();

    // Add ordered values first
    for value in &ordered_values {
        if let Some(matching_apps) = found_values.remove(value) {
            let group = build_launch_group(
                value.clone(),
                current_tag_name.clone(),
                &matching_apps,
                group_tags,
                order,
                depth,
            )?;
            groups.push(group);
        }
    }

    // Add unlisted values in an ellipsis group if any remain
    if !found_values.is_empty() {
        let mut ellipsis_apps = Vec::new();
        for apps_vec in found_values.values() {
            ellipsis_apps.extend(apps_vec.iter().copied());
        }

        let ellipsis_group = build_launch_group(
            "...".to_string(),
            current_tag_name.clone(),
            &ellipsis_apps,
            group_tags,
            order,
            depth,
        )?;
        groups.push(ellipsis_group);
    }

    Ok(groups)
}

fn build_launch_group(
    value: String,
    tag_name: String,
    apps: &[&LaunchProfile],
    group_tags: &[String],
    order: &HashMap<String, Vec<String>>,
    depth: usize,
) -> Result<LaunchGroup> {
    let is_last_level = depth == group_tags.len() - 1;
    let content = if is_last_level {
        GroupContents::Profiles(apps.iter().map(|&app| app.clone()).collect())
    } else {
        let nested = build_group_hierarchy(apps, group_tags, order, depth + 1)?;
        GroupContents::Groups(nested)
    };

    let layout_direction = if (depth & 1) == 0 {
        LayoutDirection::Horizontal
    } else {
        LayoutDirection::Vertical
    };

    Ok(LaunchGroup {
        name: value.clone(),
        tag: ScopedTag::new(tag_name, value),
        layout: layout_direction,
        content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_toml() {
        let toml = r#"
[host-1]
command = "ssh host-1"
datacenter = "ffm"

[host-2]
command = "ssh host-2"
datacenter = "ber"
        "#;

        let config: ConfigFile = toml::from_str(toml).unwrap();
        assert_eq!(config.launch_profiles.len(), 2);
        assert!(config.launch_profiles.contains_key("host-1"));
        assert!(config.launch_profiles.contains_key("host-2"));
    }

    #[test]
    fn build_app_ref_separates_tags_and_params() {
        let mut section = HashMap::new();
        section.insert("command".to_string(), str_val("ssh host-1"));
        section.insert("datacenter".to_string(), str_val("ffm"));
        section.insert("type".to_string(), str_val("router"));

        let group_tags = ["datacenter".to_string(), "type".to_string()];
        let app_ref = build_launch_profile("host-1".to_string(), section, &group_tags).unwrap();

        assert_eq!(app_ref.name, "host-1");
        assert_eq!(app_ref.params.len(), 1);
        assert_eq!(app_ref.params[0].name, "command");
        assert_eq!(app_ref.tags.len(), 2);
        assert!(app_ref.tags.iter().any(|t| t.scope == "datacenter"));
        assert!(app_ref.tags.iter().any(|t| t.scope == "type"));
    }

    #[test]
    fn hierarchy_builds_cross_product() {
        let apps = [
            app("host-1", &[], &[("datacenter", "ffm"), ("type", "router")]),
            app("host-2", &[], &[("datacenter", "ber"), ("type", "router")]),
            app("host-3", &[], &[("datacenter", "ffm"), ("type", "backend")]),
        ];
        let app_refs: Vec<_> = apps.iter().collect();

        let group_tags = ["datacenter".to_string(), "type".to_string()];
        let mut order = HashMap::new();
        order.insert(
            "datacenter".to_string(),
            vec!["ffm".to_string(), "ber".to_string()],
        );
        order.insert(
            "type".to_string(),
            vec!["router".to_string(), "backend".to_string()],
        );

        let groups = build_group_hierarchy(&app_refs, &group_tags, &order, 0).unwrap();

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "ffm");
        assert_eq!(groups[1].name, "ber");

        if let GroupContents::Groups(ref nested) = groups[0].content {
            assert_eq!(nested.len(), 2);
            assert_eq!(nested[0].name, "router");
            assert_eq!(nested[1].name, "backend");

            if let GroupContents::Profiles(ref router_apps) = nested[0].content {
                assert_eq!(router_apps.len(), 1);
                assert_eq!(router_apps[0].name, "host-1");
            } else {
                panic!("Expected Applications at leaf level");
            }
        } else {
            panic!("Expected Groups at datacenter level");
        }
    }

    #[test]
    fn unlisted_values_grouped_in_ellipsis() {
        let apps = [
            app("host-1", &[], &[("datacenter", "ffm")]),
            app("host-2", &[], &[("datacenter", "ber")]),
            app("host-3", &[], &[("datacenter", "nyc")]),
        ];
        let app_refs: Vec<_> = apps.iter().collect();

        let group_tags = ["datacenter".to_string()];
        let mut order = HashMap::new();
        order.insert(
            "datacenter".to_string(),
            vec!["ffm".to_string(), "ber".to_string()],
        );

        let groups = build_group_hierarchy(&app_refs, &group_tags, &order, 0).unwrap();

        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].name, "ffm");
        assert_eq!(groups[1].name, "ber");
        assert_eq!(groups[2].name, "...");
    }

    #[test]
    fn empty_hierarchy_when_no_groups() {
        let apps = [app("host-1", &[("command", "ssh host-1")], &[])];
        let app_refs: Vec<_> = apps.iter().collect();

        let groups = build_group_hierarchy(&app_refs, &[], &HashMap::new(), 0).unwrap();

        assert_eq!(groups.len(), 0);
    }

    #[test]
    fn value_types_converted_to_string() {
        assert_eq!(toml_value_to_string(&str_val("test")).unwrap(), "test");
        assert_eq!(toml_value_to_string(&Value::Integer(42)).unwrap(), "42");
        assert_eq!(toml_value_to_string(&Value::Float(2.5)).unwrap(), "2.5");
        assert_eq!(toml_value_to_string(&Value::Boolean(true)).unwrap(), "true");
    }

    fn app(name: &str, params: &[(&str, &str)], tags: &[(&str, &str)]) -> LaunchProfile {
        LaunchProfile {
            name: name.to_string(),
            params: Parameters(
                params
                    .iter()
                    .map(|(k, v)| Parameter::new(k.to_string(), v.to_string()))
                    .collect(),
            ),
            tags: tags
                .iter()
                .map(|(scope, tag)| ScopedTag::new(scope.to_string(), tag.to_string()))
                .collect(),
        }
    }

    fn str_val(s: &str) -> Value {
        Value::String(s.to_string())
    }
}
