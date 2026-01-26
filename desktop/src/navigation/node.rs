use massive_geometry::{Contains, Point, Rect, Vector3};
use massive_renderer::RenderGeometry;
use massive_scene::Transform;

use crate::{event_router::HitTester, focus_path::FocusPath};

#[derive(derive_more::Debug)]
pub enum NavigationNode<'a, Target> {
    Leaf {
        target: Target,
        transform: Transform,
        rect: Rect,
    },
    Container {
        // If `None`, this does not introduce another "level" when targeting nested items.
        //
        // Architecture: May combine this with rect?
        //
        // Architecture: Why then create a container at all?
        //
        // Architecture: Review the cases in which `None` is used here, this is just as a
        // convenience to return a container node instead of a list of children.
        id: Option<Target>,
        transform: Transform,
        /// This is used for deciding if nested objects are queried on hit testing. None: Query them all.
        ///
        // Architecture: This should be a sphere or AABB or something similar.
        //
        // Robustness: If `None`, this node is not considered a navigation point.
        rect: Option<Rect>,
        #[debug(skip)]
        nested: Box<dyn Fn() -> Vec<NavigationNode<'a, Target>> + 'a>,
    },
}

impl<'a, Target> NavigationNode<'a, Target> {
    pub fn with_target(mut self, target: Target) -> Self {
        match &mut self {
            NavigationNode::Leaf {
                target: leaf_target,
                ..
            } => *leaf_target = target,
            NavigationNode::Container { id, .. } => *id = Some(target),
        }
        self
    }
    pub fn with_transform(mut self, tf: Transform) -> Self {
        match &mut self {
            NavigationNode::Leaf { transform, .. } => *transform = tf,
            NavigationNode::Container { transform, .. } => *transform = tf,
        }
        self
    }

    pub fn with_rect(mut self, r: Rect) -> Self {
        match &mut self {
            NavigationNode::Leaf { rect, .. } => *rect = r,
            NavigationNode::Container { rect, .. } => *rect = Some(r),
        };
        self
    }

    pub fn map_target<NewTarget>(
        self,
        f: &'a impl Fn(Target) -> NewTarget,
    ) -> NavigationNode<'a, NewTarget>
    where
        Target: 'a,
    {
        match self {
            NavigationNode::Leaf {
                target,
                transform,
                rect,
            } => NavigationNode::Leaf {
                target: f(target),
                transform,
                rect,
            },
            NavigationNode::Container {
                id,
                transform,
                rect,
                nested,
            } => NavigationNode::Container {
                id: id.map(f),
                transform,
                rect,
                nested: Box::new(move || {
                    nested()
                        .into_iter()
                        .map(|node| node.map_target(f))
                        .collect()
                }),
            },
        }
    }
}

pub fn leaf<'a, Target>(id: impl Into<Target>, rect: Rect) -> NavigationNode<'a, Target> {
    NavigationNode::Leaf {
        target: id.into(),
        transform: Transform::IDENTITY,
        rect,
    }
}

pub fn container<'a, Target>(
    id: impl Into<Option<Target>>,
    f: impl Fn() -> Vec<NavigationNode<'a, Target>> + 'a,
) -> NavigationNode<'a, Target> {
    NavigationNode::Container {
        id: id.into(),
        transform: Transform::IDENTITY,
        rect: None,
        nested: Box::new(f),
    }
}

pub struct NavigationHitTester<'a, Target> {
    navigation: NavigationNode<'a, Target>,
    render_geometry: &'a RenderGeometry,
    base_transform: Transform,
}

impl<'a, Target> NavigationHitTester<'a, Target> {
    pub fn new(
        navigation: NavigationNode<'a, Target>,
        render_geometry: &'a RenderGeometry,
    ) -> Self {
        Self {
            navigation,
            render_geometry,
            base_transform: Transform::IDENTITY,
        }
    }
}

impl<'a, Target: Clone + PartialEq> HitTester<Target> for NavigationHitTester<'a, Target> {
    fn hit_test(&self, screen_pos: Point) -> (FocusPath<Target>, Vector3) {
        let mut hits = Vec::new();
        self.collect_hits(
            screen_pos,
            &self.navigation,
            self.base_transform,
            &mut hits,
            Vec::new(),
        );

        // Sort by z descending to get topmost hit
        hits.sort_by(|a, b| {
            b.1.z
                .partial_cmp(&a.1.z)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        hits.first()
            .map(|(path, pos)| (path.clone(), *pos))
            .unwrap_or_else(|| (FocusPath::EMPTY, screen_pos.with_z(0.0)))
    }

    fn hit_test_target(&self, screen_pos: Point, target: &FocusPath<Target>) -> Option<Vector3> {
        self.hit_test_target_recursive(screen_pos, &self.navigation, self.base_transform, target, 0)
    }
}

impl<'a, Target: Clone + PartialEq> NavigationHitTester<'a, Target> {
    fn collect_hits(
        &self,
        screen_pos: Point,
        node: &NavigationNode<'a, Target>,
        accumulated_transform: Transform,
        hits: &mut Vec<(FocusPath<Target>, Vector3)>,
        mut current_path: Vec<Target>,
    ) {
        match node {
            NavigationNode::Leaf {
                target,
                transform,
                rect,
            } => {
                let combined = accumulated_transform * *transform;
                if let Some(local_pos) = self.unproject(screen_pos, combined) {
                    let point = Point::new(local_pos.x, local_pos.y);
                    if rect.contains(point) {
                        current_path.push(target.clone());
                        hits.push((current_path.into(), local_pos));
                    }
                }
            }
            NavigationNode::Container {
                id,
                transform,
                rect,
                nested,
            } => {
                let combined = accumulated_transform * *transform;

                // Check bounds if specified, otherwise just recurse
                if let Some(bounds) = rect {
                    if let Some(local_pos) = self.unproject(screen_pos, combined) {
                        let point = Point::new(local_pos.x, local_pos.y);
                        if !bounds.contains(point) {
                            return;
                        }
                    } else {
                        return;
                    }
                }

                // Add container to path and recurse into children
                if let Some(id) = id {
                    current_path.push(id.clone());
                }
                for child in nested() {
                    self.collect_hits(screen_pos, &child, combined, hits, current_path.clone());
                }
            }
        }
    }

    fn hit_test_target_recursive(
        &self,
        screen_pos: Point,
        node: &NavigationNode<'a, Target>,
        accumulated_transform: Transform,
        target_path: &FocusPath<Target>,
        depth: usize,
    ) -> Option<Vector3> {
        if depth >= target_path.len() {
            return None;
        }

        let target = &target_path[depth];
        let is_final = depth == target_path.len() - 1;

        match node {
            NavigationNode::Leaf {
                target: leaf_target,
                transform,
                ..
            } => {
                if leaf_target == target && is_final {
                    let combined = accumulated_transform * *transform;
                    self.unproject(screen_pos, combined)
                } else {
                    None
                }
            }
            NavigationNode::Container {
                id,
                transform,
                nested,
                ..
            } => {
                if id.as_ref().is_some_and(|id| id != target) {
                    return None;
                }

                let combined = accumulated_transform * *transform;

                if is_final {
                    self.unproject(screen_pos, combined)
                } else {
                    // Continue searching in children
                    nested().into_iter().find_map(|child| {
                        self.hit_test_target_recursive(
                            screen_pos,
                            &child,
                            combined,
                            target_path,
                            depth + 1,
                        )
                    })
                }
            }
        }
    }

    fn unproject(&self, screen_pos: Point, transform: Transform) -> Option<Vector3> {
        let matrix = transform.to_matrix4();
        self.render_geometry
            .unproject_to_model_z0(screen_pos, &matrix)
    }
}
