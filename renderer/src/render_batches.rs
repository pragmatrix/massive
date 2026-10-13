use std::collections::{BTreeMap, HashMap, HashSet};

use massive_scene::Id;

use crate::renderer::RenderVisual;

/// A Z-Ordered list of Render Batches.
///
/// Why: This was introduced at the time we needed to support Z-Order / Depth Bias.
#[derive(Debug, Default)]
pub struct RenderBatches {
    /// Indexes `visuals` by their `decal_order`, because decals render one order after the other.
    decals_by_order: BTreeMap<usize, HashSet<Id>>,
    visuals: HashMap<Id, RenderVisual>,
}

impl RenderBatches {
    pub fn insert(&mut self, id: Id, render_visual: RenderVisual) {
        let order = render_visual.decal_order;
        if let Some(previous) = self.visuals.insert(id, render_visual) {
            self.unindex_decal(id, previous.decal_order);
        }
        if let Some(order) = order {
            self.decals_by_order.entry(order).or_default().insert(id);
        }
    }

    pub fn remove(&mut self, id: Id) {
        // Redundant removes might happen if a visual was inserted and removed in the same cycle.
        // So we keep this idempotent.
        if let Some(previous) = self.visuals.remove(&id) {
            self.unindex_decal(id, previous.decal_order);
        }
    }

    pub fn render_visuals(&self) -> impl Iterator<Item = &RenderVisual> {
        self.visuals.values()
    }

    pub fn normal_visuals(&self) -> impl Iterator<Item = &RenderVisual> {
        self.visuals.values().filter(|v| v.decal_order.is_none())
    }

    /// Decal visuals grouped by ascending decal order.
    pub fn decal_visuals_by_order(
        &self,
    ) -> impl Iterator<Item = impl Iterator<Item = &RenderVisual> + Clone> {
        self.decals_by_order
            .values()
            .map(|ids| ids.iter().map(|id| &self.visuals[id]))
    }

    fn unindex_decal(&mut self, id: Id, order: Option<usize>) {
        let Some(order) = order else {
            return;
        };
        let ids = self
            .decals_by_order
            .get_mut(&order)
            .expect("Internal Error: Decal order not indexed");
        assert!(ids.remove(&id), "Internal Error: Decal visual not indexed");
        if ids.is_empty() {
            self.decals_by_order.remove(&order);
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
        assert_eq!(batches.normal_visuals().count(), 1);
        assert!(batches.decals_by_order.is_empty());

        batches.insert(id, visual(location, Some(2)));
        assert_eq!(batches.normal_visuals().count(), 0);
        assert!(
            batches
                .decals_by_order
                .get(&2)
                .is_some_and(|ids| ids.contains(&id))
        );

        batches.insert(id, visual(location, Some(3)));
        assert!(!batches.decals_by_order.contains_key(&2));
        assert!(batches.decals_by_order[&3].contains(&id));

        batches.remove(id);
        assert!(batches.visuals.is_empty());
        assert!(batches.decals_by_order.is_empty());
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
