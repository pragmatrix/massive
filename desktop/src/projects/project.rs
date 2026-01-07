//! A configuration derived hierarchy with assigned ids.
use anyhow::{Context, Result, bail};
use derive_more::{Constructor, Deref};

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
    pub name: String,
    pub tag: ScopedTag,
    pub layout: LayoutDirection,
    pub contents: LaunchGroupContents,
}

#[derive(Debug)]
pub enum LaunchGroupContents {
    Groups(Vec<LaunchGroup>),
    Slots(Vec<Launcher>),
}

#[derive(Debug)]
pub struct Launcher {
    pub id: LaunchProfileId,
    pub profile: LaunchProfile,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deref, Constructor)]
pub struct GroupId(u32);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Deref, Constructor)]
pub struct LaunchProfileId(u32);

impl Project {
    pub fn from_configuration(config: ProjectConfiguration) -> Result<Self> {
        let mut group_id_counter = GroupId(1);
        let mut profile_id_counter = LaunchProfileId(1);
        let root = convert_group(config.root, &mut group_id_counter, &mut profile_id_counter);

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
}

impl LaunchGroup {
    /// Searches for a launch profile by name in this group and its descendants.
    /// Returns the LaunchProfileId if found, or an error if not found.
    fn find_profile_by_name(&self, name: &str) -> Result<LaunchProfileId> {
        match &self.contents {
            LaunchGroupContents::Slots(slots) => {
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
}

fn convert_group(
    group: configuration::LaunchGroup,
    group_id: &mut GroupId,
    profile_id: &mut LaunchProfileId,
) -> LaunchGroup {
    let id = *group_id;
    group_id.0 += 1;

    let contents = match group.content {
        GroupContents::Groups(groups) => {
            let mut converted_groups = Vec::with_capacity(groups.len());
            for child_group in groups {
                let converted = convert_group(child_group, group_id, profile_id);
                converted_groups.push(converted);
            }
            LaunchGroupContents::Groups(converted_groups)
        }
        GroupContents::Profiles(apps) => {
            let mut slots = Vec::with_capacity(apps.len());
            for app in apps {
                let slot = Launcher {
                    id: *profile_id,
                    profile: app,
                };
                profile_id.0 += 1;
                slots.push(slot);
            }
            LaunchGroupContents::Slots(slots)
        }
    };

    LaunchGroup {
        id,
        name: group.name,
        tag: group.tag,
        layout: group.layout,
        contents,
    }
}
