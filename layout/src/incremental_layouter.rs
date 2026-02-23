//! Developed together with Codex 5.3 and Claude Sonnet 4.6.
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use crate::dimensional_types::{Offset, Rect, Size};

#[cfg(test)]
use crate::LayoutAxis;
#[cfg(test)]
use crate::dimensional_types::Thickness;

pub struct IncrementalLayouter<Id, const RANK: usize>
where
    Id: Eq + Hash + Clone,
{
    root: Id,
    nodes: HashMap<Id, NodeState<Id, RANK>>,
    rects: HashMap<Id, Rect<RANK>>,
    reflow_pending: HashSet<Id>,
}

pub trait LayoutTopology<Id>
where
    Id: Eq + Hash + Clone,
{
    /// Returns `true` if the node is currently present in the topology.
    /// Used to detect removals without requiring any node data.
    fn exists(&self, id: &Id) -> bool;
    fn children_of(&self, id: &Id) -> &[Id];
    fn parent_of(&self, id: &Id) -> Option<Id>;
}

pub trait LayoutAlgorithm<Id, const RANK: usize>
where
    Id: Eq + Hash + Clone,
{
    /// Returns the outer size of `id` given its children's already-measured outer sizes.
    /// Called in post-order (all children measured before parent).
    /// For leaf nodes `child_sizes` is empty.
    fn measure(&self, id: &Id, child_sizes: &[Size<RANK>]) -> Size<RANK>;

    /// Returns one absolute child offset per entry in `child_sizes`, in the same order.
    /// `parent_offset` is the absolute position of `id`.
    /// Only called for non-leaf nodes (i.e. when the node has children).
    fn place_children(
        &self,
        id: &Id,
        parent_offset: Offset<RANK>,
        child_sizes: &[Size<RANK>],
    ) -> Vec<Offset<RANK>>;
}

impl<Id, const RANK: usize> IncrementalLayouter<Id, RANK>
where
    Id: Eq + Hash + Clone,
{
    fn invariant_violation(message: &str) -> ! {
        panic!("Internal error: {message}")
    }

    pub fn new(root: Id) -> Self {
        Self {
            root,
            nodes: HashMap::new(),
            rects: HashMap::new(),
            reflow_pending: HashSet::new(),
        }
    }

    /// Marks a node as needing reflow.
    ///
    /// Reflow-pending propagation and recompute contract:
    /// - The marked node is always included in the next recompute wave.
    /// - The full ancestor chain to the root is also included (via `collect_affected_ancestors`)
    ///   so parent sizes/positions stay coherent.
    /// - Descendants are not automatically marked pending; they are recomputed only when needed
    ///   during subtree traversal.
    pub fn mark_reflow_pending(&mut self, id: Id) {
        // Record each id once per generation; this keeps affected-collection proportional
        // to changed regions instead of all nodes.
        self.reflow_pending.insert(id.clone());
    }

    /// Recomputes layout incrementally and returns changed rectangles.
    ///
    /// Reflow changes are typically sparse, so recompute first builds an affected closure
    /// (pending nodes + ancestors) and starts only from top-most affected nodes.
    /// Pass 1 (`measure_subtree_recursive`) computes sizes bottom-up.
    /// Pass 2 (`place_subtree_recursive`) computes offsets/rects top-down.
    /// Clean branches are reused from cache and shifted when only absolute offset changed.
    ///
    /// Note on affected roots:
    /// - In a normal single-root hierarchy, collecting pending nodes + all ancestors means the
    ///   global root is in `affected`, so `collect_affected_roots` typically returns only root.
    /// - Multiple affected roots only occur if topology is disconnected/forest-like or parent
    ///   links are transiently inconsistent.
    ///
    /// Work stays proportional to changed regions while placement remains deterministic.
    pub fn recompute(
        &mut self,
        topology: &impl LayoutTopology<Id>,
        algorithm: &impl LayoutAlgorithm<Id, RANK>,
        absolute_offset: impl Into<Offset<RANK>>,
    ) -> RecomputeResult<Id, RANK> {
        let mut changed = Vec::new();
        let root = self.root.clone();
        let offset = absolute_offset.into();

        if !topology.exists(&root) {
            Self::invariant_violation("topology missing node for root");
        }

        let root_moved = self
            .rects
            .get(&root)
            .is_none_or(|current_rect| current_rect.offset != offset);
        if root_moved {
            // Root offset is an absolute input; movement should propagate even if geometry is unchanged.
            self.mark_reflow_pending(root.clone());
        }

        let pending_roots = self.refresh_pending_subtrees(topology);
        let affected = self.collect_affected_ancestors(topology, &pending_roots);
        if !affected.is_empty() {
            let affected_roots = self.collect_affected_roots(topology, &affected);
            for affected_root in affected_roots {
                let root_offset = if affected_root == root {
                    offset
                } else {
                    // Nested affected regions keep their previous absolute offset as starting point.
                    self.rects
                        .get(&affected_root)
                        .map_or(offset, |rect| rect.offset)
                };

                self.measure_subtree_recursive(algorithm, &affected_root, &affected);
                self.place_subtree_recursive(
                    algorithm,
                    &affected_root,
                    root_offset,
                    &affected,
                    &mut changed,
                );
            }
        }
        RecomputeResult { changed }
    }

    pub fn rect(&self, id: &Id) -> Option<&Rect<RANK>> {
        self.rects.get(id)
    }

    /// Decides whether the current generation should traverse into `child`.
    ///
    /// Clean children with valid cached rects are reusable during placement, so traversal is only
    /// needed for affected children or missing cache entries.
    fn should_walk_child(&self, child: &Id, affected: &HashSet<Id>) -> bool {
        // Traverse only when data is stale/missing; otherwise placement can reuse cached branch.
        affected.contains(child) || !self.rects.contains_key(child)
    }

    /// Reads the authoritative outer size for `id`.
    ///
    /// Size computations (measure accumulation and placement cursor advancement) must use
    /// `nodes.cached_outer_size` so both passes see one coherent source of truth.
    fn cached_outer_size(&self, id: &Id) -> Size<RANK> {
        self.nodes
            .get(id)
            .map(|state| state.cached_outer_size)
            .unwrap_or_else(|| {
                Self::invariant_violation("missing node for cached outer size lookup")
            })
    }

    fn evict_cached_subtree(&mut self, id: &Id) {
        if let Some(node) = self.nodes.remove(id) {
            for child in node.cached_children {
                self.evict_cached_subtree(&child);
            }
        }
        self.rects.remove(id);
    }

    /// Recursive pass 1: measure affected subtree sizes bottom-up.
    ///
    /// Parent container size depends on child outer sizes, so post-order
    /// (children before parent) is required.
    /// Sibling order is not semantically important for current size math (sum/max).
    fn measure_subtree_recursive(
        &mut self,
        algorithm: &impl LayoutAlgorithm<Id, RANK>,
        id: &Id,
        affected: &HashSet<Id>,
    ) {
        let cached_children = self
            .nodes
            .get(id)
            .map(|n| n.cached_children.clone())
            .unwrap_or_default();

        // Children must be measured before the parent (post-order).
        for child in &cached_children {
            if self.should_walk_child(child, affected) {
                self.measure_subtree_recursive(algorithm, child, affected);
            }
        }

        let child_sizes: Vec<Size<RANK>> = cached_children
            .iter()
            .map(|child| self.cached_outer_size(child))
            .collect();
        let size = algorithm.measure(id, &child_sizes);

        let node = self
            .nodes
            .get_mut(id)
            .unwrap_or_else(|| Self::invariant_violation("missing node state before measure"));
        node.cached_outer_size = size;
    }

    /// Recursive pass 2: place affected subtree top-down and shift clean cached branches.
    ///
    /// Each child absolute offset is derived from its parent's absolute offset.
    /// This pass is also where incremental reuse happens:
    /// - affected/missing children are traversed,
    /// - clean cached children are offset-shifted when needed.
    fn place_subtree_recursive(
        &mut self,
        algorithm: &impl LayoutAlgorithm<Id, RANK>,
        id: &Id,
        absolute_offset: Offset<RANK>,
        affected: &HashSet<Id>,
        changed: &mut Vec<(Id, Rect<RANK>)>,
    ) {
        let outer_size = self.cached_outer_size(id);
        self.update_rect(id, Rect::new(absolute_offset, outer_size), changed);

        let cached_children = self
            .nodes
            .get(id)
            .map(|n| n.cached_children.clone())
            .unwrap_or_default();

        if cached_children.is_empty() {
            return;
        }

        let child_sizes: Vec<Size<RANK>> = cached_children
            .iter()
            .map(|child| self.cached_outer_size(child))
            .collect();
        let child_offsets = algorithm.place_children(id, absolute_offset, &child_sizes);
        if child_offsets.len() != cached_children.len() {
            Self::invariant_violation(
                "layout algorithm returned a different number of child offsets than children",
            );
        }

        for (child, child_offset) in cached_children.iter().zip(child_offsets.iter()) {
            if self.should_walk_child(child, affected) {
                self.place_subtree_recursive(algorithm, child, *child_offset, affected, changed);
            } else {
                // Clean child: translate cached subtree if parent offset changed.
                let previous_rect = self.rects.get(child).copied().unwrap_or_else(|| {
                    Self::invariant_violation("clean child missing rect during placement")
                });
                if previous_rect.offset != *child_offset {
                    let offset_delta = Self::offset_delta(*child_offset, previous_rect.offset);
                    self.shift_subtree_recursive(child, offset_delta, changed);
                }
            }
        }
    }

    /// Shifts an already-laid-out subtree by a constant delta.
    ///
    /// When a clean subtree's size is reusable but parent placement moved, translating cached
    /// rects avoids remeasure/replacement work.
    fn shift_subtree_recursive(
        &mut self,
        id: &Id,
        offset_delta: Offset<RANK>,
        changed: &mut Vec<(Id, Rect<RANK>)>,
    ) {
        if offset_delta == Offset::ZERO {
            // Common fast path when parent move does not change this branch position.
            return;
        }

        let rect = self.rects.get(id).copied().unwrap_or_else(|| {
            Self::invariant_violation("shift traversal encountered missing rect")
        });
        let shifted = Rect::new(rect.offset + offset_delta, rect.size);
        self.update_rect(id, shifted, changed);

        let cached_children = self
            .nodes
            .get(id)
            .map(|n| n.cached_children.clone())
            .unwrap_or_default();
        for child in cached_children {
            self.shift_subtree_recursive(&child, offset_delta, changed);
        }
    }

    /// Writes rect cache and emits into `changed` only on actual value change.
    ///
    /// Callers consume `changed` as a delta stream, so suppressing equal writes avoids redundant
    /// downstream work.
    fn update_rect(&mut self, id: &Id, next_rect: Rect<RANK>, changed: &mut Vec<(Id, Rect<RANK>)>) {
        let has_changed = self
            .rects
            .get(id)
            .is_none_or(|current_rect| current_rect != &next_rect);
        if has_changed {
            self.rects.insert(id.clone(), next_rect);
            changed.push((id.clone(), next_rect));
        }
    }

    /// Step 2: collects nodes that must be recomputed for this generation.
    ///
    /// Exact definition:
    /// - Step 1 (`refresh_pending_subtrees`): refresh pending roots and descendant spec cache.
    /// - Step 2: from remaining pending roots, include each node and all ancestors up to root.
    /// - Do not include descendants unless they are independently pending.
    ///
    /// Consequence in a connected single-root tree: `affected` contains root whenever any node is
    /// pending.
    fn collect_affected_ancestors(
        &self,
        topology: &impl LayoutTopology<Id>,
        pending_nodes: &[Id],
    ) -> HashSet<Id> {
        let mut affected = HashSet::new();

        for pending_id in pending_nodes {
            let mut current = Some(pending_id.clone());
            while let Some(node_id) = current {
                if !affected.insert(node_id.clone()) {
                    // Ancestor chain already merged into closure.
                    break;
                }

                current = topology.parent_of(&node_id);
            }
        }

        affected
    }

    /// Step 1: drains pending ids, refreshes cached children for pending roots and descendants,
    /// and evicts stale caches for removed nodes.
    ///
    /// Returns pending roots that should seed affected-ancestor collection.
    fn refresh_pending_subtrees(&mut self, topology: &impl LayoutTopology<Id>) -> Vec<Id> {
        let pending_nodes = std::mem::take(&mut self.reflow_pending);
        let mut pending_roots = Vec::with_capacity(pending_nodes.len());
        let mut refreshed = HashSet::new();

        for pending_id in pending_nodes {
            if !topology.exists(&pending_id) {
                self.evict_cached_subtree(&pending_id);
                continue;
            }

            self.refresh_children_subtree(topology, &pending_id, &mut refreshed);
            pending_roots.push(pending_id);
        }

        pending_roots
    }

    fn refresh_children_subtree(
        &mut self,
        topology: &impl LayoutTopology<Id>,
        id: &Id,
        refreshed: &mut HashSet<Id>,
    ) {
        if !refreshed.insert(id.clone()) {
            return;
        }

        let current_children = topology.children_of(id).to_vec();
        let removed_cached_children = self.nodes.get(id).map_or_else(Vec::new, |node| {
            node.cached_children
                .iter()
                .filter(|child| !topology.exists(child))
                .cloned()
                .collect::<Vec<Id>>()
        });

        for removed_child in removed_cached_children {
            self.evict_cached_subtree(&removed_child);
        }

        match self.nodes.entry(id.clone()) {
            Entry::Occupied(mut occupied) => {
                let node = occupied.get_mut();
                node.cached_children = current_children.clone();
            }
            Entry::Vacant(vacant) => {
                vacant.insert(NodeState::new(current_children.clone()));
            }
        }

        for child in &current_children {
            if !topology.exists(child) {
                Self::invariant_violation("topology missing child during refresh");
            }
            self.refresh_children_subtree(topology, child, refreshed);
        }
    }

    /// Selects the minimal set of affected nodes that should start subtree traversals.
    ///
    /// Given `affected` (= pending nodes plus their ancestors), this returns only those affected
    /// nodes whose parent is either:
    /// - missing (`None`), or
    /// - not itself in `affected`.
    ///
    /// In other words, each returned id is the topmost node of one connected affected region.
    /// Recompute can then run exactly one measure/place traversal per region instead of starting
    /// from every affected node (which would duplicate work on overlapping ancestor chains).
    ///
    /// In a connected single-root hierarchy where `affected` is built by ancestor-closure,
    /// this will typically return exactly one id: the global root.
    fn collect_affected_roots(
        &self,
        topology: &impl LayoutTopology<Id>,
        affected: &HashSet<Id>,
    ) -> Vec<Id> {
        affected
            .iter()
            .filter(|id| {
                topology
                    .parent_of(id)
                    .is_none_or(|parent| !affected.contains(&parent))
            })
            .cloned()
            .collect()
    }

    fn offset_delta(new_offset: Offset<RANK>, previous_offset: Offset<RANK>) -> Offset<RANK> {
        let mut delta = Offset::ZERO;
        for dim in 0..RANK {
            delta[dim] = new_offset[dim] - previous_offset[dim];
        }
        delta
    }
}

#[derive(Debug, Clone)]
struct NodeState<Id, const RANK: usize>
where
    Id: Eq + Hash + Clone,
{
    cached_outer_size: Size<RANK>,
    cached_children: Vec<Id>,
}

impl<Id, const RANK: usize> NodeState<Id, RANK>
where
    Id: Eq + Hash + Clone,
{
    fn new(cached_children: Vec<Id>) -> Self {
        Self {
            cached_outer_size: Size::EMPTY,
            cached_children,
        }
    }
}

#[derive(Debug)]
pub struct RecomputeResult<Id: Clone, const RANK: usize> {
    pub changed: Vec<(Id, Rect<RANK>)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::max;
    use std::collections::{HashMap, HashSet};

    #[derive(Default)]
    struct TestTopology {
        nodes: HashSet<usize>,
        children: HashMap<usize, Vec<usize>>,
        parent: HashMap<usize, usize>,
        panic_on_missing_node_parent_lookup: bool,
    }

    impl LayoutTopology<usize> for TestTopology {
        fn exists(&self, id: &usize) -> bool {
            self.nodes.contains(id)
        }

        fn children_of(&self, id: &usize) -> &[usize] {
            self.children.get(id).map(Vec::as_slice).unwrap_or(&[])
        }

        fn parent_of(&self, id: &usize) -> Option<usize> {
            if self.panic_on_missing_node_parent_lookup && !self.nodes.contains(id) {
                panic!("parent_of called for id without node");
            }
            self.parent.get(id).copied()
        }
    }

    struct ContainerSpec {
        layout_axis: LayoutAxis,
        padding: Thickness<2>,
        spacing: u32,
    }

    impl ContainerSpec {
        fn new(layout_axis: LayoutAxis) -> Self {
            Self {
                layout_axis,
                padding: Thickness::ZERO,
                spacing: 0,
            }
        }
    }

    #[derive(Default)]
    struct TestAlgorithm {
        leaf_sizes: HashMap<usize, Size<2>>,
        container_specs: HashMap<usize, ContainerSpec>,
    }

    impl LayoutAlgorithm<usize, 2> for TestAlgorithm {
        fn measure(&self, id: &usize, child_sizes: &[Size<2>]) -> Size<2> {
            if let Some(&size) = self.leaf_sizes.get(id) {
                return size;
            }
            let spec = self
                .container_specs
                .get(id)
                .unwrap_or_else(|| panic!("missing container spec for node {id}"));
            let axis = *spec.layout_axis;
            let padding = spec.padding;
            let spacing = spec.spacing;
            let mut inner_size = Size::EMPTY;
            for (index, &child_size) in child_sizes.iter().enumerate() {
                for dim in 0..2 {
                    if dim == axis {
                        inner_size[dim] += child_size[dim];
                        if index > 0 {
                            inner_size[dim] += spacing;
                        }
                    } else {
                        inner_size[dim] = max(inner_size[dim], child_size[dim]);
                    }
                }
            }
            padding.leading + inner_size + padding.trailing
        }

        fn place_children(
            &self,
            id: &usize,
            parent_offset: Offset<2>,
            child_sizes: &[Size<2>],
        ) -> Vec<Offset<2>> {
            let spec = self
                .container_specs
                .get(id)
                .unwrap_or_else(|| panic!("missing container spec for node {id}"));
            let axis = *spec.layout_axis;
            let padding = spec.padding;
            let spacing = spec.spacing;
            let mut cursor: Offset<2> = padding.leading.into();
            let mut offsets = Vec::with_capacity(child_sizes.len());
            for (index, &child_size) in child_sizes.iter().enumerate() {
                if index > 0 {
                    cursor[axis] += spacing as i32;
                }
                offsets.push(parent_offset + cursor);
                cursor[axis] += child_size[axis] as i32;
            }
            offsets
        }
    }

    impl TestTopology {
        fn insert_node(&mut self, id: usize) {
            self.nodes.insert(id);
        }

        fn remove_node(&mut self, id: usize) -> Vec<usize> {
            let parent = self.parent.get(&id).copied();
            if let Some(parent_id) = parent
                && let Some(siblings) = self.children.get_mut(&parent_id)
            {
                siblings.retain(|child| child != &id);
            }
            self.parent.remove(&id);

            let mut removed = vec![id];
            let mut index = 0;
            while index < removed.len() {
                let current = removed[index];
                self.nodes.remove(&current);
                if let Some(children) = self.children.remove(&current) {
                    for child in children {
                        self.parent.remove(&child);
                        removed.push(child);
                    }
                }
                index += 1;
            }

            let mut affected = Vec::new();
            if let Some(parent_id) = parent {
                affected.push(parent_id);
            }
            affected.sort_unstable();
            affected.dedup();
            affected
        }

        fn set_children(&mut self, id: usize, children: Vec<usize>) -> Vec<usize> {
            let old_children = self
                .children
                .insert(id, children.clone())
                .unwrap_or_default();
            let child_set: HashSet<usize> = children.iter().copied().collect();
            let mut dirty_parents = vec![id];

            for old_child in old_children {
                if !child_set.contains(&old_child)
                    && self
                        .parent
                        .get(&old_child)
                        .is_some_and(|parent| *parent == id)
                {
                    self.parent.remove(&old_child);
                }
            }

            for child in children {
                if let Some(previous_parent) = self.parent.insert(child, id)
                    && previous_parent != id
                {
                    if let Some(previous_siblings) = self.children.get_mut(&previous_parent) {
                        previous_siblings.retain(|sibling| sibling != &child);
                    }
                    dirty_parents.push(previous_parent);
                }
            }

            dirty_parents.sort_unstable();
            dirty_parents.dedup();
            dirty_parents
        }
    }

    impl TestAlgorithm {
        fn upsert_leaf(&mut self, id: usize, size: impl Into<Size<2>>) {
            self.leaf_sizes.insert(id, size.into());
        }

        fn upsert_container(&mut self, id: usize, layout_axis: LayoutAxis) {
            self.container_specs
                .insert(id, ContainerSpec::new(layout_axis));
        }

        fn set_padding(&mut self, id: usize, padding: impl Into<Thickness<2>>) -> bool {
            if let Some(spec) = self.container_specs.get_mut(&id) {
                spec.padding = padding.into();
                return true;
            }
            false
        }

        fn set_spacing(&mut self, id: usize, spacing: u32) -> bool {
            if let Some(spec) = self.container_specs.get_mut(&id) {
                spec.spacing = spacing;
                return true;
            }
            false
        }

        fn set_leaf_size(&mut self, id: usize, size: impl Into<Size<2>>) -> bool {
            if let Some(s) = self.leaf_sizes.get_mut(&id) {
                *s = size.into();
                return true;
            }
            false
        }

        fn remove_node(&mut self, id: usize) {
            self.leaf_sizes.remove(&id);
            self.container_specs.remove(&id);
        }
    }

    #[test]
    fn initial_recompute_emits_changed_then_stabilizes() {
        let mut topology = TestTopology::default();
        let mut algorithm = TestAlgorithm::default();
        let mut layouter = IncrementalLayouter::<usize, 2>::new(0);
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            0,
            LayoutAxis::HORIZONTAL,
        );
        upsert_leaf(&mut layouter, &mut topology, &mut algorithm, 1, [10, 10]);
        set_children(&mut layouter, &mut topology, 0, vec![1]);

        let first = layouter.recompute(&topology, &algorithm, [0, 0]);
        assert_eq!(first.changed.len(), 2);

        let second = layouter.recompute(&topology, &algorithm, [0, 0]);
        assert!(second.changed.is_empty());
    }

    #[test]
    fn changing_leaf_size_updates_leaf_and_ancestor_rects() {
        let mut topology = TestTopology::default();
        let mut algorithm = TestAlgorithm::default();
        let mut layouter = IncrementalLayouter::<usize, 2>::new(0);
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            0,
            LayoutAxis::HORIZONTAL,
        );
        upsert_leaf(&mut layouter, &mut topology, &mut algorithm, 1, [10, 10]);
        set_children(&mut layouter, &mut topology, 0, vec![1]);

        let _ = layouter.recompute(&topology, &algorithm, [0, 0]);

        set_intrinsic_size(&mut layouter, &mut algorithm, 1, [20, 10]);
        let update = layouter.recompute(&topology, &algorithm, [0, 0]);

        assert_eq!(update.changed.len(), 2);
        assert_eq!(
            layouter.rect(&0).map(|rect| rect.size),
            Some(Size::from([20, 10]))
        );
        assert_eq!(
            layouter.rect(&1).map(|rect| rect.size),
            Some(Size::from([20, 10]))
        );
    }

    #[test]
    fn remove_node_clears_cached_rect_on_recompute() {
        let mut topology = TestTopology::default();
        let mut algorithm = TestAlgorithm::default();
        let mut layouter = IncrementalLayouter::<usize, 2>::new(0);
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            0,
            LayoutAxis::HORIZONTAL,
        );
        upsert_leaf(&mut layouter, &mut topology, &mut algorithm, 1, [10, 10]);
        set_children(&mut layouter, &mut topology, 0, vec![1]);
        let _ = layouter.recompute(&topology, &algorithm, [0, 0]);

        remove_node(&mut layouter, &mut topology, &mut algorithm, 1);
        set_children(&mut layouter, &mut topology, 0, vec![]);
        let _ = layouter.recompute(&topology, &algorithm, [0, 0]);

        assert!(layouter.rect(&1).is_none());
        assert_eq!(
            layouter.rect(&0).map(|rect| rect.size),
            Some(Size::from([0, 0]))
        );
    }

    #[test]
    fn changing_one_branch_does_not_emit_unaffected_sibling_branch() {
        let mut topology = TestTopology::default();
        let mut algorithm = TestAlgorithm::default();
        let mut layouter = IncrementalLayouter::<usize, 2>::new(0);
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            0,
            LayoutAxis::VERTICAL,
        );
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            10,
            LayoutAxis::HORIZONTAL,
        );
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            20,
            LayoutAxis::HORIZONTAL,
        );
        upsert_leaf(&mut layouter, &mut topology, &mut algorithm, 1, [10, 10]);
        upsert_leaf(&mut layouter, &mut topology, &mut algorithm, 2, [8, 20]);

        set_children(&mut layouter, &mut topology, 10, vec![1]);
        set_children(&mut layouter, &mut topology, 20, vec![2]);
        set_children(&mut layouter, &mut topology, 0, vec![10, 20]);

        let _ = layouter.recompute(&topology, &algorithm, [0, 0]);

        set_intrinsic_size(&mut layouter, &mut algorithm, 1, [12, 10]);
        let update = layouter.recompute(&topology, &algorithm, [0, 0]);

        let mut changed_ids: Vec<usize> = update.changed.into_iter().map(|(id, _)| id).collect();
        changed_ids.sort_unstable();

        assert_eq!(changed_ids, vec![0, 1, 10]);
        assert!(layouter.rect(&20).is_some());
        assert!(layouter.rect(&2).is_some());
    }

    #[test]
    fn reparenting_child_detaches_from_previous_parent() {
        let mut topology = TestTopology::default();
        let mut algorithm = TestAlgorithm::default();
        let mut layouter = IncrementalLayouter::<usize, 2>::new(0);
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            0,
            LayoutAxis::HORIZONTAL,
        );
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            10,
            LayoutAxis::VERTICAL,
        );
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            20,
            LayoutAxis::VERTICAL,
        );
        upsert_leaf(&mut layouter, &mut topology, &mut algorithm, 1, [7, 5]);

        set_children(&mut layouter, &mut topology, 10, vec![1]);
        set_children(&mut layouter, &mut topology, 0, vec![10, 20]);
        let _ = layouter.recompute(&topology, &algorithm, [0, 0]);

        set_children(&mut layouter, &mut topology, 20, vec![1]);
        let _ = layouter.recompute(&topology, &algorithm, [0, 0]);

        assert_eq!(
            layouter.rect(&10).map(|rect| rect.size),
            Some(Size::from([0, 0]))
        );
        assert_eq!(
            layouter.rect(&20).map(|rect| rect.size),
            Some(Size::from([7, 5]))
        );
        assert_eq!(
            layouter.rect(&1).map(|rect| rect.offset),
            Some(Offset::from([0, 0]))
        );
    }

    #[test]
    fn root_offset_change_updates_offsets() {
        let mut topology = TestTopology::default();
        let mut algorithm = TestAlgorithm::default();
        let mut layouter = IncrementalLayouter::<usize, 2>::new(0);
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            0,
            LayoutAxis::HORIZONTAL,
        );
        upsert_leaf(&mut layouter, &mut topology, &mut algorithm, 1, [10, 5]);
        set_children(&mut layouter, &mut topology, 0, vec![1]);

        let _ = layouter.recompute(&topology, &algorithm, [0, 0]);
        let moved = layouter.recompute(&topology, &algorithm, [8, 13]);

        assert_eq!(moved.changed.len(), 2);
        assert_eq!(
            layouter.rect(&0).map(|rect| rect.offset),
            Some([8, 13].into())
        );
        assert_eq!(
            layouter.rect(&1).map(|rect| rect.offset),
            Some([8, 13].into())
        );
        assert_eq!(
            layouter.rect(&0).map(|rect| rect.size),
            Some([10, 5].into())
        );
        assert_eq!(
            layouter.rect(&1).map(|rect| rect.size),
            Some([10, 5].into())
        );
    }

    #[test]
    fn spacing_and_padding_affect_layout_geometry() {
        let mut topology = TestTopology::default();
        let mut algorithm = TestAlgorithm::default();
        let mut layouter = IncrementalLayouter::<usize, 2>::new(0);
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            0,
            LayoutAxis::HORIZONTAL,
        );
        upsert_leaf(&mut layouter, &mut topology, &mut algorithm, 1, [10, 5]);
        upsert_leaf(&mut layouter, &mut topology, &mut algorithm, 2, [7, 4]);
        set_children(&mut layouter, &mut topology, 0, vec![1, 2]);
        set_padding(&mut layouter, &mut algorithm, 0, ([3, 2], [4, 1]));
        set_spacing(&mut layouter, &mut algorithm, 0, 6);

        let _ = layouter.recompute(&topology, &algorithm, [0, 0]);

        assert_eq!(
            layouter.rect(&1).map(|rect| rect.offset),
            Some([3, 2].into())
        );
        assert_eq!(
            layouter.rect(&2).map(|rect| rect.offset),
            Some([19, 2].into())
        );
        assert_eq!(
            layouter.rect(&0).map(|rect| rect.size),
            Some([30, 8].into())
        );
    }

    #[test]
    #[should_panic]
    fn removing_root_panics() {
        let mut topology = TestTopology::default();
        let mut algorithm = TestAlgorithm::default();
        let mut layouter = IncrementalLayouter::<usize, 2>::new(0);
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            0,
            LayoutAxis::HORIZONTAL,
        );
        upsert_leaf(&mut layouter, &mut topology, &mut algorithm, 1, [10, 5]);
        set_children(&mut layouter, &mut topology, 0, vec![1]);
        let _ = layouter.recompute(&topology, &algorithm, [0, 0]);

        remove_node(&mut layouter, &mut topology, &mut algorithm, 0);
        let _ = layouter.recompute(&topology, &algorithm, [0, 0]);
    }

    #[test]
    fn removing_pending_node_is_ignored_on_recompute() {
        let mut topology = TestTopology::default();
        let mut algorithm = TestAlgorithm::default();
        let mut layouter = IncrementalLayouter::<usize, 2>::new(0);
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            0,
            LayoutAxis::HORIZONTAL,
        );
        upsert_leaf(&mut layouter, &mut topology, &mut algorithm, 1, [10, 5]);
        set_children(&mut layouter, &mut topology, 0, vec![1]);
        let _ = layouter.recompute(&topology, &algorithm, [0, 0]);

        layouter.mark_reflow_pending(1);
        remove_node(&mut layouter, &mut topology, &mut algorithm, 1);
        set_children(&mut layouter, &mut topology, 0, vec![]);

        let update = layouter.recompute(&topology, &algorithm, [0, 0]);

        assert!(layouter.rect(&1).is_none());
        assert!(update.changed.iter().any(|(id, _)| id == &0));
    }

    #[test]
    fn removing_pending_node_does_not_query_missing_parent() {
        let mut topology = TestTopology::default();
        let mut algorithm = TestAlgorithm::default();
        let mut layouter = IncrementalLayouter::<usize, 2>::new(0);
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            0,
            LayoutAxis::HORIZONTAL,
        );
        upsert_leaf(&mut layouter, &mut topology, &mut algorithm, 1, [10, 5]);
        set_children(&mut layouter, &mut topology, 0, vec![1]);
        let _ = layouter.recompute(&topology, &algorithm, [0, 0]);

        layouter.mark_reflow_pending(1);
        remove_node(&mut layouter, &mut topology, &mut algorithm, 1);
        set_children(&mut layouter, &mut topology, 0, vec![]);
        topology.panic_on_missing_node_parent_lookup = true;

        let update = layouter.recompute(&topology, &algorithm, [0, 0]);

        assert!(layouter.rect(&1).is_none());
        assert!(update.changed.iter().any(|(id, _)| id == &0));
    }

    #[test]
    #[should_panic]
    fn missing_child_node_panics_on_recompute() {
        let mut topology = TestTopology::default();
        let mut algorithm = TestAlgorithm::default();
        let mut layouter = IncrementalLayouter::<usize, 2>::new(0);
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            0,
            LayoutAxis::HORIZONTAL,
        );
        set_children(&mut layouter, &mut topology, 0, vec![99]);

        let _ = layouter.recompute(&topology, &algorithm, [0, 0]);
    }

    struct BrokenPlaceChildrenAlgorithm<'a> {
        inner: &'a TestAlgorithm,
    }

    impl LayoutAlgorithm<usize, 2> for BrokenPlaceChildrenAlgorithm<'_> {
        fn measure(&self, id: &usize, child_sizes: &[Size<2>]) -> Size<2> {
            self.inner.measure(id, child_sizes)
        }

        fn place_children(
            &self,
            _id: &usize,
            _parent_offset: Offset<2>,
            _child_sizes: &[Size<2>],
        ) -> Vec<Offset<2>> {
            Vec::new()
        }
    }

    #[test]
    #[should_panic]
    fn place_children_offset_count_mismatch_panics() {
        let mut topology = TestTopology::default();
        let mut algorithm = TestAlgorithm::default();
        let mut layouter = IncrementalLayouter::<usize, 2>::new(0);
        upsert_container(
            &mut layouter,
            &mut topology,
            &mut algorithm,
            0,
            LayoutAxis::HORIZONTAL,
        );
        upsert_leaf(&mut layouter, &mut topology, &mut algorithm, 1, [10, 5]);
        upsert_leaf(&mut layouter, &mut topology, &mut algorithm, 2, [7, 4]);
        set_children(&mut layouter, &mut topology, 0, vec![1, 2]);

        let broken = BrokenPlaceChildrenAlgorithm { inner: &algorithm };
        let _ = layouter.recompute(&topology, &broken, [0, 0]);
    }

    fn upsert_leaf(
        layouter: &mut IncrementalLayouter<usize, 2>,
        topology: &mut TestTopology,
        algorithm: &mut TestAlgorithm,
        id: usize,
        size: impl Into<Size<2>>,
    ) {
        topology.insert_node(id);
        algorithm.upsert_leaf(id, size);
        layouter.mark_reflow_pending(id);
    }

    fn upsert_container(
        layouter: &mut IncrementalLayouter<usize, 2>,
        topology: &mut TestTopology,
        algorithm: &mut TestAlgorithm,
        id: usize,
        layout_axis: LayoutAxis,
    ) {
        topology.insert_node(id);
        algorithm.upsert_container(id, layout_axis);
        layouter.mark_reflow_pending(id);
    }

    fn set_children(
        layouter: &mut IncrementalLayouter<usize, 2>,
        topology: &mut TestTopology,
        id: usize,
        children: Vec<usize>,
    ) {
        for node_id in topology.set_children(id, children) {
            layouter.mark_reflow_pending(node_id);
        }
    }

    fn set_intrinsic_size(
        layouter: &mut IncrementalLayouter<usize, 2>,
        algorithm: &mut TestAlgorithm,
        id: usize,
        size: impl Into<Size<2>>,
    ) {
        if algorithm.set_leaf_size(id, size) {
            layouter.mark_reflow_pending(id);
        }
    }

    fn set_padding(
        layouter: &mut IncrementalLayouter<usize, 2>,
        algorithm: &mut TestAlgorithm,
        id: usize,
        padding: impl Into<Thickness<2>>,
    ) {
        if algorithm.set_padding(id, padding) {
            layouter.mark_reflow_pending(id);
        }
    }

    fn set_spacing(
        layouter: &mut IncrementalLayouter<usize, 2>,
        algorithm: &mut TestAlgorithm,
        id: usize,
        spacing: u32,
    ) {
        if algorithm.set_spacing(id, spacing) {
            layouter.mark_reflow_pending(id);
        }
    }

    fn remove_node(
        layouter: &mut IncrementalLayouter<usize, 2>,
        topology: &mut TestTopology,
        algorithm: &mut TestAlgorithm,
        id: usize,
    ) {
        for node_id in topology.remove_node(id) {
            layouter.mark_reflow_pending(node_id);
        }
        algorithm.remove_node(id);
    }
}
