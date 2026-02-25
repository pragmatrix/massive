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

#[cfg(test)]
mod tests {
    use massive_scene::{Id, id_generator};

    use super::RenderBatches;
    use crate::renderer::{PipelineBatches, RenderVisual};

    #[test]
    fn insert_move_and_remove_keeps_storage_consistent() {
        let mut batches = RenderBatches::default();
        let id = new_id();
        let location = new_id();

        batches.insert(id, visual(location, None));
        assert_eq!(batches.visuals_to_decal_order.get(&id), Some(&None));
        assert!(batches.normal_visuals.contains_key(&id));
        assert!(batches.decal_visuals_by_order.is_empty());

        batches.insert(id, visual(location, Some(2)));
        assert_eq!(batches.visuals_to_decal_order.get(&id), Some(&Some(2)));
        assert!(!batches.normal_visuals.contains_key(&id));
        assert!(
            batches
                .decal_visuals_by_order
                .get(&2)
                .is_some_and(|m| m.contains_key(&id))
        );

        batches.remove(id);
        assert!(!batches.visuals_to_decal_order.contains_key(&id));
        assert!(!batches.normal_visuals.contains_key(&id));
        assert!(!batches.decal_visuals_by_order.contains_key(&2));
    }

    #[test]
    fn same_order_update_replaces_visual_in_place() {
        let mut batches = RenderBatches::default();
        let id = new_id();
        let location_a = new_id();
        let location_b = new_id();

        batches.insert(id, visual(location_a, Some(4)));
        batches.insert(id, visual(location_b, Some(4)));

        let updated = batches
            .decal_visuals_by_order
            .get(&4)
            .and_then(|m| m.get(&id))
            .expect("expected visual in decal order bucket");

        assert_eq!(updated.location_id, location_b);
        assert_eq!(batches.visuals_to_decal_order.get(&id), Some(&Some(4)));
    }

    fn new_id() -> Id {
        id_generator::acquire::<RenderBatches>()
    }

    fn visual(location_id: Id, decal_order: Option<usize>) -> RenderVisual {
        RenderVisual {
            location_id,
            decal_order,
            clip_bounds: None,
            batches: PipelineBatches::new(0),
        }
    }
}
