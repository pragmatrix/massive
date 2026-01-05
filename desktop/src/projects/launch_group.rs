//! A configuration derived hierarchy with assigned ids.
//!
// Architecture: Can't the configuration reader directly assign ids?
use derive_more::{Constructor, Deref};

use crate::projects::configuration::{
    self, GroupContents, LaunchProfile, LayoutDirection, ScopedTag,
};

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
    pub id: LauncherId,
    pub profile: LaunchProfile,
}

#[derive(Debug, Copy, Clone, Constructor, PartialEq, Eq, Hash, Deref)]
pub struct GroupId(u32);

#[derive(Debug, Copy, Clone, Constructor, PartialEq, Eq, Hash, Deref)]
pub struct LauncherId(u32);

impl LaunchGroup {
    pub fn from_configuration(group: configuration::LaunchGroup) -> Self {
        let mut group_id_counter = GroupId(1);
        let mut launcher_id_counter = LauncherId(1);
        convert_group(group, &mut group_id_counter, &mut launcher_id_counter)
    }
}

fn convert_group(
    group: configuration::LaunchGroup,
    group_id: &mut GroupId,
    slot_id: &mut LauncherId,
) -> LaunchGroup {
    let id = *group_id;
    group_id.0 += 1;

    let contents = match group.content {
        GroupContents::Groups(groups) => {
            let mut converted_groups = Vec::with_capacity(groups.len());
            for child_group in groups {
                let converted = convert_group(child_group, group_id, slot_id);
                converted_groups.push(converted);
            }
            LaunchGroupContents::Groups(converted_groups)
        }
        GroupContents::LaunchProfiles(apps) => {
            let mut slots = Vec::with_capacity(apps.len());
            for app in apps {
                let slot = Launcher {
                    id: *slot_id,
                    profile: app,
                };
                slot_id.0 += 1;
                slots.push(slot);
            }
            LaunchGroupContents::Slots(slots)
        }
    };

    LaunchGroup {
        id,
        name: group.name,
        tag: group.tag,
        layout: group.direction,
        contents,
    }
}
