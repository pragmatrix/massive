use derive_more::{Constructor, Deref};

use crate::projects::configuration::{
    GroupContents, LaunchGroup, LaunchProfile, LayoutDirection, ScopedTag,
};

#[derive(Debug)]
pub struct SlotGroup {
    pub id: GroupId,
    pub name: String,
    pub tag: ScopedTag,
    pub layout: LayoutDirection,
    pub contents: SlotGroupContents,
}

#[derive(Debug)]
pub enum SlotGroupContents {
    Groups(Vec<SlotGroup>),
    Slots(Vec<SlotDef>),
}

#[derive(Debug)]
pub struct SlotDef {
    pub id: SlotId,
    pub application: LaunchProfile,
}

#[derive(Debug, Copy, Clone, Constructor, PartialEq, Eq, Hash, Deref)]
pub struct GroupId(u32);

#[derive(Debug, Copy, Clone, Constructor, PartialEq, Eq, Hash, Deref)]
pub struct SlotId(u32);

impl SlotGroup {
    pub fn from_configuration(group: LaunchGroup) -> Self {
        let mut group_id_counter = GroupId(1);
        let mut slot_id_counter = SlotId(1);
        convert_group(group, &mut group_id_counter, &mut slot_id_counter)
    }
}

fn convert_group(group: LaunchGroup, group_id: &mut GroupId, slot_id: &mut SlotId) -> SlotGroup {
    let id = *group_id;
    group_id.0 += 1;

    let contents = match group.content {
        GroupContents::Groups(groups) => {
            let mut converted_groups = Vec::with_capacity(groups.len());
            for child_group in groups {
                let converted = convert_group(child_group, group_id, slot_id);
                converted_groups.push(converted);
            }
            SlotGroupContents::Groups(converted_groups)
        }
        GroupContents::LaunchProfiles(apps) => {
            let mut slots = Vec::with_capacity(apps.len());
            for app in apps {
                let slot = SlotDef {
                    id: *slot_id,
                    application: app,
                };
                slot_id.0 += 1;
                slots.push(slot);
            }
            SlotGroupContents::Slots(slots)
        }
    };

    SlotGroup {
        id,
        name: group.name,
        tag: group.tag,
        layout: group.direction,
        contents,
    }
}
