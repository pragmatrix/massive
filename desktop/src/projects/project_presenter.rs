use std::collections::HashMap;

use crate::projects::slot_group::{GroupId, SlotGroup, SlotId};

#[derive(Debug)]
struct ProjectPresenter {
    /// The current hierarchy, directly derived from the configuration. This is for layout. It
    /// references the presenters through GroupIds and SlotIds.
    hierarchy: SlotGroup,

    groups: HashMap<GroupId, GroupPresenter>,
    // Naming: Find a better name for Slot
    slots: HashMap<SlotId, SlotPresenter>,
}

#[derive(Debug)]
#[allow(dead_code)]
struct SlotPresenter {}

#[derive(Debug)]
struct GroupPresenter {}
