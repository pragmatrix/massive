use std::collections::HashMap;

use massive_geometry::{Point, Transform};
use massive_layout::{
    LayoutAlgorithm, LayoutTopology, Offset, Placement, Rect as LayoutRect, Size as LayoutSize,
};

use super::DesktopTarget;
use crate::OrderedHierarchy;
use crate::hit_tester::PlacementSource;

#[derive(Debug, Clone)]
pub(super) struct MeasureOutcome {
    pub(super) size_changed: bool,
    pub(super) parent: Option<DesktopTarget>,
}

#[derive(Debug, Clone, Copy)]
enum LayoutEntry {
    Measured { size: LayoutSize<2> },
    Placed { placement: Placement<Transform, 2> },
}

impl LayoutEntry {
    fn size(self) -> LayoutSize<2> {
        match self {
            Self::Measured { size } => size,
            Self::Placed { placement } => placement.rect.size,
        }
    }

    fn placement(self) -> Option<Placement<Transform, 2>> {
        match self {
            Self::Measured { .. } => None,
            Self::Placed { placement } => Some(placement),
        }
    }
}

pub(super) struct DesktopLayoutState {
    entries: HashMap<DesktopTarget, LayoutEntry>,
}

impl DesktopLayoutState {
    pub(super) fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub(super) fn measure_node(
        &mut self,
        target: &DesktopTarget,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
    ) -> MeasureOutcome {
        let child_sizes: Vec<_> = topology
            .children_of(target)
            .iter()
            .map(|child| {
                self.entries
                    .get(child)
                    .copied()
                    .unwrap_or_else(|| {
                        panic!("Internal error: child should be measured before parent")
                    })
                    .size()
            })
            .collect();

        let measured = algorithm.measure(target, &child_sizes);
        let size_changed = self
            .entries
            .get(target)
            .is_none_or(|current| current.size() != measured);
        if size_changed {
            self.entries
                .insert(target.clone(), LayoutEntry::Measured { size: measured });
        }

        MeasureOutcome {
            size_changed,
            parent: topology.parent_of(target),
        }
    }

    pub(super) fn missing_child_measures(
        &self,
        target: &DesktopTarget,
        topology: &impl LayoutTopology<DesktopTarget>,
    ) -> Vec<DesktopTarget> {
        topology
            .children_of(target)
            .iter()
            .filter(|child| !self.entries.contains_key(*child))
            .cloned()
            .collect()
    }

    pub(super) fn place_children_of(
        &mut self,
        target: &DesktopTarget,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
    ) -> Vec<DesktopTarget> {
        if !topology.exists(target) {
            return Vec::new();
        }

        if *target == DesktopTarget::Desktop {
            let size = self
                .entries
                .get(target)
                .expect("Internal error: missing measured layout size for desktop root")
                .size();
            self.entries.insert(
                target.clone(),
                LayoutEntry::Placed {
                    placement: Placement::new(
                        Transform::default(),
                        LayoutRect::new(Offset::default(), size),
                    ),
                },
            );
        }

        let mut changed_targets = Vec::new();
        for (target, placement) in self.place_children(target, topology, algorithm) {
            let is_changed = self
                .entries
                .get(&target)
                .copied()
                .and_then(LayoutEntry::placement)
                .is_none_or(|current| current != placement);
            if is_changed {
                self.entries
                    .insert(target.clone(), LayoutEntry::Placed { placement });
                changed_targets.push(target);
            }
        }

        changed_targets
    }

    pub(super) fn remove_subtree(
        &mut self,
        target: &DesktopTarget,
        topology: &impl LayoutTopology<DesktopTarget>,
    ) {
        let mut stack = vec![target.clone()];
        while let Some(current) = stack.pop() {
            stack.extend(topology.children_of(&current).iter().cloned());
            self.entries.remove(&current);
        }
    }

    fn place_children(
        &self,
        target: &DesktopTarget,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
    ) -> Vec<(DesktopTarget, Placement<Transform, 2>)> {
        let children = topology.children_of(target);
        if children.is_empty() {
            return Vec::new();
        }

        let child_sizes: Vec<_> = children
            .iter()
            .map(|child| {
                self.entries
                    .get(child)
                    .copied()
                    .expect("Internal error: missing measured layout size for child")
                    .size()
            })
            .collect();

        let child_transforms = algorithm.place_children(target, &child_sizes);
        if child_transforms.len() != children.len() {
            panic!("Internal error: child placement count does not match child count")
        }

        let mut placements = Vec::with_capacity(children.len());
        for index in 0..children.len() {
            let child = &children[index];
            let size = child_sizes[index];
            let child_transform = &child_transforms[index];
            placements.push((
                child.clone(),
                Placement::new(
                    child_transform.transform,
                    LayoutRect::new(child_transform.offset, size),
                ),
            ));
        }

        placements
    }

    pub(super) fn local_placement(
        &self,
        target: &DesktopTarget,
    ) -> Option<Placement<Transform, 2>> {
        self.entries
            .get(target)
            .copied()
            .and_then(LayoutEntry::placement)
    }

    pub(super) fn absolute_placement(
        &self,
        target: &DesktopTarget,
        topology: &impl LayoutTopology<DesktopTarget>,
    ) -> Option<Placement<Transform, 2>> {
        let mut path = Vec::new();
        let mut current = target.clone();
        loop {
            path.push(current.clone());
            let Some(parent) = topology.parent_of(&current) else {
                break;
            };
            current = parent;
        }

        let mut origin_transform = Transform::default();
        let mut offset = Offset::default();
        for path_target in path.iter().rev() {
            let placement = self.local_placement(path_target)?;
            let local_origin_transform = if *path_target == DesktopTarget::Desktop {
                // Desktop transform is already origin-based (IDENTITY in the common case).
                placement.transform
            } else {
                let local_center = Self::layout_local_center(placement.rect.size);
                placement.transform.to_origin_space(local_center)
            };
            origin_transform *= local_origin_transform;
            offset += placement.rect.offset;
        }

        let local = self.local_placement(target)?;
        let local_center = Self::layout_local_center(local.rect.size);
        let transform = origin_transform.to_anchor_space(local_center);

        Some(Placement::new(
            transform,
            LayoutRect::new(offset, local.rect.size),
        ))
    }

    fn layout_local_center(size: LayoutSize<2>) -> Point {
        Point::new(size[0] as f64 * 0.5, size[1] as f64 * 0.5)
    }
}

impl PlacementSource for DesktopLayoutState {
    fn placement(
        &self,
        target: &DesktopTarget,
        hierarchy: &OrderedHierarchy<DesktopTarget>,
    ) -> Option<Placement<Transform, 2>> {
        self.absolute_placement(target, hierarchy)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use massive_geometry::Transform;
    use massive_layout::{
        LayoutAlgorithm, LayoutTopology, Offset, Size as LayoutSize, TransformOffset,
    };

    use super::DesktopLayoutState;
    use crate::desktop_system::DesktopTarget;
    use crate::projects::GroupId;

    #[derive(Default)]
    struct TestTopology {
        nodes: HashSet<DesktopTarget>,
        children: HashMap<DesktopTarget, Vec<DesktopTarget>>,
        parent: HashMap<DesktopTarget, DesktopTarget>,
    }

    impl TestTopology {
        fn insert_node(&mut self, node: DesktopTarget) {
            self.nodes.insert(node);
        }

        fn set_children(&mut self, parent: DesktopTarget, children: Vec<DesktopTarget>) {
            for child in &children {
                self.parent.insert(child.clone(), parent.clone());
                self.nodes.insert(child.clone());
            }
            self.nodes.insert(parent.clone());
            self.children.insert(parent, children);
        }
    }

    impl LayoutTopology<DesktopTarget> for TestTopology {
        fn exists(&self, id: &DesktopTarget) -> bool {
            self.nodes.contains(id)
        }

        fn children_of(&self, id: &DesktopTarget) -> &[DesktopTarget] {
            self.children.get(id).map(Vec::as_slice).unwrap_or(&[])
        }

        fn parent_of(&self, id: &DesktopTarget) -> Option<DesktopTarget> {
            self.parent.get(id).cloned()
        }
    }

    struct TestAlgorithm {
        child_offset: Offset<2>,
        mismatch_child_count: bool,
    }

    impl LayoutAlgorithm<DesktopTarget, Transform, 2> for TestAlgorithm {
        fn measure(&self, id: &DesktopTarget, child_sizes: &[LayoutSize<2>]) -> LayoutSize<2> {
            match id {
                DesktopTarget::Desktop => {
                    let width = child_sizes.iter().map(|size| size[0]).sum::<u32>();
                    let height = child_sizes.iter().map(|size| size[1]).max().unwrap_or(0);
                    [width, height].into()
                }
                _ => [10, 5].into(),
            }
        }

        fn place_children(
            &self,
            _id: &DesktopTarget,
            child_sizes: &[LayoutSize<2>],
        ) -> Vec<TransformOffset<Transform, 2>> {
            if self.mismatch_child_count {
                return Vec::new();
            }

            child_sizes
                .iter()
                .map(|_| TransformOffset::new(Transform::default(), self.child_offset))
                .collect()
        }
    }

    #[test]
    #[should_panic(expected = "Internal error: child placement count does not match child count")]
    fn place_children_count_mismatch_panics() {
        let mut state = DesktopLayoutState::new();
        let mut topology = TestTopology::default();

        let group = DesktopTarget::Group(GroupId::new());
        topology.insert_node(DesktopTarget::Desktop);
        topology.set_children(DesktopTarget::Desktop, vec![group.clone()]);

        let algorithm = TestAlgorithm {
            child_offset: Offset::default(),
            mismatch_child_count: true,
        };

        state.measure_node(&group, &topology, &algorithm);
        state.measure_node(&DesktopTarget::Desktop, &topology, &algorithm);

        state.place_children_of(&DesktopTarget::Desktop, &topology, &algorithm);
    }

    #[test]
    #[should_panic(expected = "Internal error: child should be measured before parent")]
    fn measure_parent_without_child_measure_panics() {
        let mut state = DesktopLayoutState::new();
        let mut topology = TestTopology::default();

        let group = DesktopTarget::Group(GroupId::new());
        topology.insert_node(DesktopTarget::Desktop);
        topology.set_children(DesktopTarget::Desktop, vec![group]);

        let algorithm = TestAlgorithm {
            child_offset: Offset::default(),
            mismatch_child_count: false,
        };

        state.measure_node(&DesktopTarget::Desktop, &topology, &algorithm);
    }

    #[test]
    fn changed_targets_returned_from_placement_update() {
        let mut state = DesktopLayoutState::new();
        let mut topology = TestTopology::default();

        let group = DesktopTarget::Group(GroupId::new());
        topology.insert_node(DesktopTarget::Desktop);
        topology.set_children(DesktopTarget::Desktop, vec![group.clone()]);

        let initial_algorithm = TestAlgorithm {
            child_offset: [0, 0].into(),
            mismatch_child_count: false,
        };

        state.measure_node(&group, &topology, &initial_algorithm);
        state.measure_node(&DesktopTarget::Desktop, &topology, &initial_algorithm);
        state.place_children_of(&DesktopTarget::Desktop, &topology, &initial_algorithm);

        let updated_algorithm = TestAlgorithm {
            child_offset: [7, 3].into(),
            mismatch_child_count: false,
        };
        let changed =
            state.place_children_of(&DesktopTarget::Desktop, &topology, &updated_algorithm);

        assert_eq!(changed, vec![group.clone()]);

        let final_group = state
            .local_placement(&group)
            .expect("expected final group placement");

        assert_eq!(final_group.rect.offset, [7, 3].into());
    }
}
