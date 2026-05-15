use std::collections::HashMap;

use massive_geometry::{Point, Transform, Vector3};
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

pub(super) struct DesktopLayoutState {
    measured_sizes: HashMap<DesktopTarget, LayoutSize<2>>,
    local_placements: HashMap<DesktopTarget, Placement<Transform, 2>>,
}

impl DesktopLayoutState {
    pub(super) fn new() -> Self {
        Self {
            measured_sizes: HashMap::new(),
            local_placements: HashMap::new(),
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
            .filter(|child| !self.measured_sizes.contains_key(*child))
            .cloned()
            .collect()
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
                *self.measured_sizes.get(child).unwrap_or_else(|| {
                    panic!("Internal error: child should be measured before parent")
                })
            })
            .collect();

        let measured = algorithm.measure(target, &child_sizes);
        let size_changed = self
            .measured_sizes
            .get(target)
            .is_none_or(|current| current != &measured);
        self.measured_sizes.insert(target.clone(), measured);

        MeasureOutcome {
            size_changed,
            parent: topology.parent_of(target),
        }
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

        self.update_root_placement(target);

        let mut changed_targets = Vec::new();
        for (target, placement) in self.place_children(target, topology, algorithm) {
            let is_changed = self
                .local_placements
                .get(&target)
                .is_none_or(|current| current != &placement);
            self.local_placements.insert(target.clone(), placement);
            if is_changed {
                changed_targets.push(target);
            }
        }

        self.local_placements
            .retain(|target, _| topology.exists(target));
        self.measured_sizes
            .retain(|target, _| topology.exists(target));

        changed_targets
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
                *self
                    .measured_sizes
                    .get(child)
                    .expect("Internal error: missing measured layout size for child")
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
        self.local_placements.get(target).copied()
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
            let placement = self.local_placements.get(path_target)?;
            let local_origin_transform = if *path_target == DesktopTarget::Desktop {
                // Desktop transform is already origin-based (IDENTITY in the common case).
                placement.transform
            } else {
                let local_center = Self::layout_local_center(placement.rect.size);
                Self::transform_with_layout(placement.transform, local_center)
            };
            origin_transform *= local_origin_transform;
            offset += placement.rect.offset;
        }

        let local = self.local_placements.get(target)?;
        let local_center = Self::layout_local_center(local.rect.size);
        let local_center = Vector3::new(local_center.x, local_center.y, 0.0);
        let center_translation = origin_transform.translate
            + origin_transform.rotate * (local_center * origin_transform.scale);

        let transform = Transform::new(
            center_translation,
            origin_transform.rotate,
            origin_transform.scale,
        );

        Some(Placement::new(
            transform,
            LayoutRect::new(offset, local.rect.size),
        ))
    }

    fn layout_local_center(size: LayoutSize<2>) -> Point {
        Point::new(size[0] as f64 * 0.5, size[1] as f64 * 0.5)
    }

    fn transform_with_layout(layout_transform: Transform, local_center: Point) -> Transform {
        let local_center = Vector3::new(local_center.x, local_center.y, 0.0);
        let origin_translation =
            layout_transform.translate + layout_transform.rotate * -local_center;
        Transform::new(
            origin_translation,
            layout_transform.rotate,
            layout_transform.scale,
        )
    }

    fn update_root_placement(&mut self, target: &DesktopTarget) {
        if *target != DesktopTarget::Desktop {
            return;
        }

        let size = *self
            .measured_sizes
            .get(target)
            .expect("Internal error: missing measured layout size for desktop root");
        self.local_placements.insert(
            target.clone(),
            Placement::new(
                Transform::default(),
                LayoutRect::new(Offset::default(), size),
            ),
        );
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
