use std::collections::HashMap;

use massive_geometry::{Point, Transform};
use massive_layout::{
    LayoutAlgorithm, LayoutTopology, MeasuredLayout, Offset, Placement, Rect as LayoutRect,
    Size as LayoutSize,
};

use super::DesktopTarget;
use crate::OrderedHierarchy;
use crate::hit_tester::PlacementSource;

#[derive(Debug, Clone)]
pub struct MeasureOutcome {
    pub size_changed: bool,
    pub parent: Option<DesktopTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementUpdate {
    Unchanged,
    ChangedSizeUnchanged,
    ChangedSizeChanged,
}

#[derive(Debug, Clone, Copy)]
struct LayoutEntry {
    measured: MeasuredLayout<2>,
    placement: Option<Placement<Transform, 2>>,
}

pub struct DesktopLayoutState {
    entries: HashMap<DesktopTarget, LayoutEntry>,
}

impl DesktopLayoutState {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn measure_node(
        &mut self,
        target: &DesktopTarget,
        topology: &impl LayoutTopology<DesktopTarget>,
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
    ) -> MeasureOutcome {
        let child_measurements: Vec<_> = topology
            .children_of(target)
            .iter()
            .map(|child| {
                self.entries
                    .get(child)
                    .copied()
                    .unwrap_or_else(|| {
                        panic!("Internal error: child should be measured before parent")
                    })
                    .measured
            })
            .collect();

        let measured = algorithm.measure(target, &child_measurements);
        let current_entry = self.entries.get(target).copied();
        let size_changed = current_entry.is_none_or(|current| current.measured != measured);
        if size_changed {
            self.entries.insert(
                target.clone(),
                LayoutEntry {
                    measured,
                    placement: current_entry.and_then(|entry| entry.placement),
                },
            );
        }

        MeasureOutcome {
            size_changed,
            parent: topology.parent_of(target),
        }
    }

    pub fn missing_child_measures(
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

    // Place the children of a given target, and return a list of size changes if there are any. For
    // example, for children that expand to fill their parent.
    pub fn place_children_of(
        &mut self,
        target: &DesktopTarget,
        children: &[DesktopTarget],
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
    ) -> Vec<PlacementUpdate> {
        if *target == DesktopTarget::Desktop {
            let size = self
                .entries
                .get(target)
                .expect("Internal error: missing measured layout size for desktop root")
                .measured
                .size;
            let placement = Placement::new(
                Transform::default(),
                LayoutRect::new(Offset::default(), size),
            );
            self.entries.insert(
                target.clone(),
                LayoutEntry {
                    measured: size.into(),
                    placement: Some(placement),
                },
            );
        }

        if children.is_empty() {
            return Vec::new();
        }

        let child_placements = self.place_children(target, children, algorithm);

        // Update the placements, and see if there are size changes.
        let mut updates = Vec::with_capacity(children.len());
        for (child, placement) in children.iter().zip(child_placements) {
            let Some(entry) = self.entries.get_mut(child) else {
                panic!("Internal error: missing measured layout entry for child")
            };
            let current_placement = entry.placement;
            let is_changed = current_placement.is_none_or(|current| current != placement);
            if is_changed {
                let size_changed = current_placement
                    .is_none_or(|current| current.rect.size != placement.rect.size);
                entry.placement = Some(placement);
                updates.push(match size_changed {
                    true => PlacementUpdate::ChangedSizeChanged,
                    false => PlacementUpdate::ChangedSizeUnchanged,
                });
            } else {
                updates.push(PlacementUpdate::Unchanged);
            }
        }

        updates
    }

    pub fn remove_subtree(
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
        children: &[DesktopTarget],
        algorithm: &impl LayoutAlgorithm<DesktopTarget, Transform, 2>,
    ) -> Vec<Placement<Transform, 2>> {
        let child_measurements: Vec<_> = children
            .iter()
            .map(|child| {
                self.entries
                    .get(child)
                    .copied()
                    .expect("Internal error: missing measured layout size for child")
                    .measured
            })
            .collect();

        let parent_entry = self
            .entries
            .get(target)
            .expect("Internal error: missing layout entry for parent");
        let parent_size = parent_entry
            .placement
            .map(|placement| placement.rect.size)
            .unwrap_or(parent_entry.measured.size);
        let child_placements = algorithm.place_children(target, parent_size, &child_measurements);
        if child_placements.len() != children.len() {
            panic!("Internal error: child placement count does not match child count")
        }

        child_placements
    }

    pub fn absolute_placement(
        &self,
        target: &DesktopTarget,
        topology: &impl LayoutTopology<DesktopTarget>,
    ) -> Placement<Transform, 2> {
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
        let mut visible = true;
        for path_target in path.iter().rev() {
            let placement = self.local_placement(path_target);
            visible = visible && placement.visible;
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

        let local = self.local_placement(target);
        let local_center = Self::layout_local_center(local.rect.size);
        let transform = origin_transform.to_anchor_space(local_center);

        Placement::new(transform, LayoutRect::new(offset, local.rect.size)).with_visibility(visible)
    }

    fn layout_local_center(size: LayoutSize<2>) -> Point {
        Point::new(size[0] as f64 * 0.5, size[1] as f64 * 0.5)
    }

    pub fn local_placement(&self, target: &DesktopTarget) -> Placement<Transform, 2> {
        self.entries
            .get(target)
            .copied()
            .unwrap_or_else(|| {
                panic!("Internal error: missing layout entry for target")
            })
            .placement
            .unwrap_or_else(|| {
                panic!("Internal error: missing local placement for target")
            })
    }
}

impl PlacementSource for DesktopLayoutState {
    fn placement(
        &self,
        target: &DesktopTarget,
        hierarchy: &OrderedHierarchy<DesktopTarget>,
    ) -> Placement<Transform, 2> {
        self.absolute_placement(target, hierarchy)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use massive_geometry::Transform;
    use massive_layout::{
        LayoutAlgorithm, LayoutTopology, MeasuredLayout, Offset, Placement, Rect as LayoutRect,
        Size as LayoutSize,
    };

    use super::{DesktopLayoutState, PlacementUpdate};
    use crate::desktop_system::DesktopTarget;
    use crate::projects::ProjectId;

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
        fn measure(
            &self,
            id: &DesktopTarget,
            child_measurements: &[MeasuredLayout<2>],
        ) -> MeasuredLayout<2> {
            let size: LayoutSize<2> = match id {
                DesktopTarget::Desktop => {
                    let width = child_measurements
                        .iter()
                        .map(|child| child.size[0])
                        .sum::<u32>();
                    let height = child_measurements
                        .iter()
                        .map(|child| child.size[1])
                        .max()
                        .unwrap_or(0);
                    [width, height].into()
                }
                _ => [10, 5].into(),
            };
            size.into()
        }

        fn place_children(
            &self,
            _id: &DesktopTarget,
            _parent_size: LayoutSize<2>,
            child_measurements: &[MeasuredLayout<2>],
        ) -> Vec<Placement<Transform, 2>> {
            if self.mismatch_child_count {
                return Vec::new();
            }

            child_measurements
                .iter()
                .map(|child| {
                    Placement::new(
                        Transform::default(),
                        LayoutRect::new(self.child_offset, child.size),
                    )
                })
                .collect()
        }
    }

    #[test]
    #[should_panic(expected = "Internal error: child placement count does not match child count")]
    fn place_children_count_mismatch_panics() {
        let mut state = DesktopLayoutState::new();
        let mut topology = TestTopology::default();

        let project = DesktopTarget::Project(ProjectId::new());
        topology.insert_node(DesktopTarget::Desktop);
        topology.set_children(DesktopTarget::Desktop, vec![project.clone()]);

        let algorithm = TestAlgorithm {
            child_offset: Offset::default(),
            mismatch_child_count: true,
        };

        state.measure_node(&project, &topology, &algorithm);
        state.measure_node(&DesktopTarget::Desktop, &topology, &algorithm);

        state.place_children_of(
            &DesktopTarget::Desktop,
            topology.children_of(&DesktopTarget::Desktop),
            &algorithm,
        );
    }

    #[test]
    #[should_panic(expected = "Internal error: child should be measured before parent")]
    fn measure_parent_without_child_measure_panics() {
        let mut state = DesktopLayoutState::new();
        let mut topology = TestTopology::default();

        let project = DesktopTarget::Project(ProjectId::new());
        topology.insert_node(DesktopTarget::Desktop);
        topology.set_children(DesktopTarget::Desktop, vec![project]);

        let algorithm = TestAlgorithm {
            child_offset: Offset::default(),
            mismatch_child_count: false,
        };

        state.measure_node(&DesktopTarget::Desktop, &topology, &algorithm);
    }

    #[test]
    fn changed_children_returned_from_placement_update() {
        let mut state = DesktopLayoutState::new();
        let mut topology = TestTopology::default();

        let project = DesktopTarget::Project(ProjectId::new());
        topology.insert_node(DesktopTarget::Desktop);
        topology.set_children(DesktopTarget::Desktop, vec![project.clone()]);

        let initial_algorithm = TestAlgorithm {
            child_offset: [0, 0].into(),
            mismatch_child_count: false,
        };

        state.measure_node(&project, &topology, &initial_algorithm);
        state.measure_node(&DesktopTarget::Desktop, &topology, &initial_algorithm);
        state.place_children_of(
            &DesktopTarget::Desktop,
            topology.children_of(&DesktopTarget::Desktop),
            &initial_algorithm,
        );

        let updated_algorithm = TestAlgorithm {
            child_offset: [7, 3].into(),
            mismatch_child_count: false,
        };
        let changed = state.place_children_of(
            &DesktopTarget::Desktop,
            topology.children_of(&DesktopTarget::Desktop),
            &updated_algorithm,
        );

        assert_eq!(changed, vec![PlacementUpdate::ChangedSizeUnchanged]);

        let final_project = state.local_placement(&project);

        assert_eq!(final_project.rect.offset, [7, 3].into());
    }
}
