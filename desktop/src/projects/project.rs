//! A configuration derived hierarchy with assigned ids.
use anyhow::{Context, Result, bail};
use uuid::Uuid;

use crate::projects::configuration::{
    self, GroupContents, LaunchProfile, LayoutDirection, ProjectConfiguration, ScopedTag,
};

#[derive(Debug)]
pub struct Project {
    pub start: Option<LaunchProfileId>,
    pub root: LaunchGroup,
}

#[derive(Debug)]
pub struct LaunchGroup {
    pub id: GroupId,
    pub properties: LaunchGroupProperties,
    pub contents: LaunchGroupContents,
}

#[allow(unused)]
#[derive(Debug, Clone)]
pub struct LaunchGroupProperties {
    pub name: String,
    pub tag: ScopedTag,
    pub layout: LayoutDirection,
}

#[derive(Debug)]
pub enum LaunchGroupContents {
    Groups(Vec<LaunchGroup>),
    Launchers(Vec<Launcher>),
}

impl LaunchGroupContents {
    #[allow(unused)]
    pub fn len(&self) -> usize {
        match self {
            LaunchGroupContents::Groups(launch_groups) => launch_groups.len(),
            LaunchGroupContents::Launchers(launchers) => launchers.len(),
        }
    }
}

#[derive(Debug)]
pub struct Launcher {
    pub id: LaunchProfileId,
    pub profile: LaunchProfile,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct GroupId(Uuid);

impl GroupId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct LaunchProfileId(Uuid);

impl LaunchProfileId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Project {
    pub fn from_configuration(config: ProjectConfiguration) -> Result<Self> {
        let root = convert_group(config.root);

        let start = match config.startup {
            Some(profile_name) => {
                Some(root.find_profile_by_name(&profile_name).with_context(|| {
                    format!(
                        "Startup profile '{}' not found in configuration",
                        profile_name
                    )
                })?)
            }
            None => None,
        };

        Ok(Project { start, root })
    }

    #[allow(unused)]
    pub fn get_launch_profile(&self, id: LaunchProfileId) -> Option<&LaunchProfile> {
        self.root.get_launch_profile(id)
    }
}

impl LaunchGroup {
    /// Searches for a launch profile by name in this group and its descendants.
    /// Returns the LaunchProfileId if found, or an error if not found.
    fn find_profile_by_name(&self, name: &str) -> Result<LaunchProfileId> {
        match &self.contents {
            LaunchGroupContents::Launchers(slots) => {
                for launcher in slots {
                    if launcher.profile.name == name {
                        return Ok(launcher.id);
                    }
                }
                bail!("Launch profile '{}' not found", name)
            }
            LaunchGroupContents::Groups(groups) => {
                for group in groups {
                    if let Ok(id) = group.find_profile_by_name(name) {
                        return Ok(id);
                    }
                }
                bail!("Launch profile '{}' not found", name)
            }
        }
    }

    fn get_launch_profile(&self, id: LaunchProfileId) -> Option<&LaunchProfile> {
        match &self.contents {
            LaunchGroupContents::Launchers(launchers) => launchers
                .iter()
                .find(|launcher| launcher.id == id)
                .map(|launcher| &launcher.profile),
            LaunchGroupContents::Groups(groups) => {
                for group in groups {
                    if let Some(profile) = group.get_launch_profile(id) {
                        return Some(profile);
                    }
                }
                None
            }
        }
    }

    /// Returns an ASCII tree visualization of the group hierarchy.
    #[allow(unused)]
    pub fn visualize(&self) -> String {
        let mut output = String::new();
        self.visualize_impl(&mut output, "", true, true);
        output
    }

    fn visualize_impl(&self, output: &mut String, prefix: &str, is_last: bool, is_root: bool) {
        let (connector, extension) = if is_root {
            ("", "")
        } else if is_last {
            ("└── ", "    ")
        } else {
            ("├── ", "│   ")
        };

        output.push_str(prefix);
        output.push_str(connector);
        output.push_str(&self.properties.name);
        output.push('\n');

        let child_prefix = format!("{}{}", prefix, extension);

        match &self.contents {
            LaunchGroupContents::Groups(groups) => {
                for (i, group) in groups.iter().enumerate() {
                    let is_last_child = i == groups.len() - 1;
                    group.visualize_impl(output, &child_prefix, is_last_child, false);
                }
            }
            LaunchGroupContents::Launchers(launchers) => {
                for (i, launcher) in launchers.iter().enumerate() {
                    let is_last_child = i == launchers.len() - 1;
                    let connector = if is_last_child {
                        "└── "
                    } else {
                        "├── "
                    };
                    output.push_str(&child_prefix);
                    output.push_str(connector);
                    output.push_str(&launcher.profile.name);
                    output.push('\n');
                }
            }
        }
    }
}

fn convert_group(group: configuration::LaunchGroupSpec) -> LaunchGroup {
    let id = GroupId::new();

    let contents = match group.content {
        GroupContents::Groups(groups) => {
            let mut converted_groups = Vec::with_capacity(groups.len());
            for child_group in groups {
                let converted = convert_group(child_group);
                converted_groups.push(converted);
            }
            LaunchGroupContents::Groups(converted_groups)
        }
        GroupContents::Profiles(profiles) => {
            let mut slots = Vec::with_capacity(profiles.len());
            for profile in profiles {
                let slot = Launcher {
                    id: LaunchProfileId::new(),
                    profile,
                };
                slots.push(slot);
            }
            LaunchGroupContents::Launchers(slots)
        }
    };

    LaunchGroup {
        id,
        properties: LaunchGroupProperties {
            name: group.name,
            tag: group.tag,
            layout: group.layout,
        },
        contents,
    }
}
