use std::collections::{BTreeMap, HashMap, btree_map, hash_map};

use massive_scene::Id;

use crate::renderer::RenderVisual;

/// A Z-Ordered list of Render Batches.
///
/// Why: This was introduced at the time we needed to support Z-Order / Depth Bias.
#[derive(Debug, Default)]
pub struct RenderBatches {
    visuals_to_decal_order: HashMap<Id, Option<usize>>,
    pub normal_visuals: HashMap<Id, RenderVisual>,
    pub decal_visuals_by_order: BTreeMap<usize, HashMap<Id, RenderVisual>>,
}

impl RenderBatches {
    pub fn insert(&mut self, id: Id, render_visual: RenderVisual) {
        let order = render_visual.decal_order;
        match self.visuals_to_decal_order.entry(id) {
            hash_map::Entry::Vacant(vacant_entry) => {
                vacant_entry.insert(order);
                self.insert_new(id, render_visual);
            }
            hash_map::Entry::Occupied(occupied_entry) => {
                if *occupied_entry.get() == order {
                    self.update_existing(id, render_visual);
                } else {
                    // Depth changed, re-insert.
                    self.remove(id);
                    self.insert(id, render_visual);
                }
            }
        }
    }

    pub fn remove(&mut self, id: Id) {
        let Some(order) = self.visuals_to_decal_order.remove(&id) else {
            // Redundant removes might happen if a visual was inserted and removed in the same
            // cycle. So we keep this idempotent.
            return;
        };

        self.remove_with_order(id, order)
    }

    fn remove_with_order(&mut self, id: Id, order: Option<usize>) {
        match order {
            Some(order) => {
                let btree_map::Entry::Occupied(mut entry) =
                    self.decal_visuals_by_order.entry(order)
                else {
                    panic!("Internal Error: Decal order not found");
                };
                let map = entry.get_mut();
                map.remove(&id)
                    .expect("Internal Error: Visual not found in decal order map");
                if map.is_empty() {
                    entry.remove();
                }
            }
            None => {
                self.normal_visuals
                    .remove(&id)
                    .expect("Internal Error: Visual not found in normal visuals");
            }
        }
    }

    pub fn render_visuals(&self) -> impl Iterator<Item = &RenderVisual> {
        self.normal_visuals.values().chain(
            self.decal_visuals_by_order
                .values()
                .flat_map(|v| v.values()),
        )
    }

    fn insert_new(&mut self, id: Id, render_visual: RenderVisual) {
        let order = render_visual.decal_order;
        match order {
            Some(order) => {
                self.decal_visuals_by_order
                    .entry(order)
                    .or_default()
                    .insert(id, render_visual);
            }
            None => {
                self.normal_visuals.insert(id, render_visual);
            }
        }
    }

    fn update_existing(&mut self, id: Id, render_visual: RenderVisual) {
        let order = render_visual.decal_order;
        match order {
            Some(order) => {
                let btree_map::Entry::Occupied(mut entry) =
                    self.decal_visuals_by_order.entry(order)
                else {
                    panic!("Internal Error: Decal order not found");
                };
                let map = entry.get_mut();
                map.insert(id, render_visual)
                    .expect("Internal Error: Visual not found in decal order map");
            }
            None => {
                self.normal_visuals
                    .insert(id, render_visual)
                    .expect("Internal Error: Visual not found in normal visuals");
            }
        }
    }
}
