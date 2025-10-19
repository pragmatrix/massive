use std::collections::{BTreeMap, HashMap, btree_map, hash_map};

use massive_scene::Id;

use crate::renderer::RenderVisual;

/// A Z-Ordered list of Render Batches.
///
/// Why: This was introduced at the time we needed to support Z-Order / Depth Bias.
#[derive(Debug, Default)]
pub struct RenderBatches {
    /// Per visual location and pipeline batches.
    visuals_to_depth: HashMap<Id, usize>,
    // Performance: Use a HashMap here, and sort the depths on demand.
    pub by_depth_bias: BTreeMap<usize, HashMap<Id, RenderVisual>>,
}

impl RenderBatches {
    pub fn insert(&mut self, id: Id, render_visual: RenderVisual) {
        let depth = render_visual.depth_bias;
        match self.visuals_to_depth.entry(id) {
            hash_map::Entry::Vacant(vacant_entry) => {
                vacant_entry.insert(depth);
                self.by_depth_bias
                    .entry(depth)
                    .or_default()
                    .insert(id, render_visual);
            }
            hash_map::Entry::Occupied(occupied_entry) => {
                if *occupied_entry.get() == depth {
                    self.by_depth_bias
                        .get_mut(&depth)
                        .expect("Internal error: Depth batches missing")
                        .insert(id, render_visual);
                } else {
                    // Depth changed, re-insert.
                    self.remove(id);
                    self.insert(id, render_visual);
                }
            }
        }
    }

    pub fn remove(&mut self, id: Id) {
        let depth = self
            .visuals_to_depth
            .remove(&id)
            .expect("Internal Error: Visual not found");
        let btree_map::Entry::Occupied(mut entry) = self.by_depth_bias.entry(depth) else {
            panic!("Internal Error: Depth not found");
        };
        let map = entry.get_mut();
        map.remove(&id)
            .expect("Internal Error: Visual not found in depth map");
        if map.is_empty() {
            entry.remove();
        }
    }

    pub fn render_visuals(&self) -> impl Iterator<Item = &RenderVisual> {
        self.by_depth_bias.values().flat_map(|v| v.values())
    }
}
