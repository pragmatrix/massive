//! A configuration derived hierarchy with assigned ids.
use anyhow::{Context, Result, bail};
use derive_more::{From, Into};
use uuid::Uuid;

use crate::projects::configuration::{
    self, GroupChildSpec, LaunchProfile, LayoutDirection, ProjectConfiguration,
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
    pub children: Vec<LaunchGroupChild>,
}

#[allow(unused)]
#[derive(Debug, Clone)]
pub struct LaunchGroupProperties {
    pub name: String,
    pub layout: LayoutDirection,
}

#[derive(Debug)]
pub enum LaunchGroupChild {
    Group(LaunchGroup),
    Launcher(Launcher),
}

#[derive(Debug)]
pub struct Launcher {
    pub id: LaunchProfileId,
    pub profile: LaunchProfile,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From, Into)]
pub struct GroupId(Uuid);

impl GroupId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, From, Into)]
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
        for child in &self.children {
            match child {
                LaunchGroupChild::Launcher(launcher) => {
                    if launcher.profile.name == name {
                        return Ok(launcher.id);
                    }
                }
                LaunchGroupChild::Group(group) => {
                    if let Ok(id) = group.find_profile_by_name(name) {
                        return Ok(id);
                    }
                }
            }
        }

        bail!("Launch profile '{}' not found", name)
    }

    fn get_launch_profile(&self, id: LaunchProfileId) -> Option<&LaunchProfile> {
        for child in &self.children {
            match child {
                LaunchGroupChild::Launcher(launcher) => {
                    if launcher.id == id {
                        return Some(&launcher.profile);
                    }
                }
                LaunchGroupChild::Group(group) => {
                    if let Some(profile) = group.get_launch_profile(id) {
                        return Some(profile);
                    }
                }
            }
        }

        None
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

        for (index, child) in self.children.iter().enumerate() {
            let is_last_child = index == self.children.len() - 1;

            match child {
                LaunchGroupChild::Group(group) => {
                    group.visualize_impl(output, &child_prefix, is_last_child, false);
                }
                LaunchGroupChild::Launcher(launcher) => {
                    let connector = if is_last_child { "└── " } else { "├── " };
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

    let mut children = Vec::with_capacity(group.children.len());
    for child in group.children {
        match child {
            GroupChildSpec::Group(child_group) => {
                children.push(LaunchGroupChild::Group(convert_group(child_group)));
            }
            GroupChildSpec::Launcher(profile) => {
                children.push(LaunchGroupChild::Launcher(Launcher {
                    id: LaunchProfileId::new(),
                    profile,
                }));
            }
        }
    }

    LaunchGroup {
        id,
        properties: LaunchGroupProperties {
            name: group.name,
            layout: group.layout,
        },
        children,
    }
}
