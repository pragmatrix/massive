use std::{cmp::max, ops::Add};

use derive_more::{Constructor, Deref, From, Into};

#[derive(Debug, Copy, Clone, From, Into, Deref)]
pub struct LayoutAxis(usize);

pub enum LayoutInfo<const RANK: usize> {
    Container {
        layout_axis: LayoutAxis,
        child_count: usize,
    },
    Leaf {
        size: Size<RANK>,
    },
}

pub trait LayoutNode<const RANK: usize> {
    fn layout_info(&self) -> LayoutInfo<RANK>;
    fn get_child_mut(&mut self, index: usize) -> &mut dyn LayoutNode<RANK>;
    fn set_rect(&mut self, rect: Rect<RANK>);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size<const RANK: usize> {
    pub dim: [u32; RANK],
}

impl<const RANK: usize> Size<RANK> {
    pub const ZERO: Self = Self { dim: [0u32; RANK] };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offset<const RANK: usize> {
    pub dim: [u32; RANK],
}

impl<const RANK: usize> Add for Offset<RANK> {
    type Output = Self;

    fn add(mut self, rhs: Self) -> Self {
        for i in 0..RANK {
            self.dim[i] += rhs.dim[i]
        }
        self
    }
}

impl<const RANK: usize> Offset<RANK> {
    pub const ZERO: Self = Self { dim: [0u32; RANK] };
}

#[derive(Debug, Copy, Clone, Constructor, PartialEq, Eq)]
// Optimization: Would it be enough to just track the intermediate Sizes that grow in layout direction?
pub struct Rect<const RANK: usize> {
    pub pos: Offset<RANK>,
    // We also track sizes, so that we can compute the final rects.
    pub size: Size<RANK>,
}

impl<const RANK: usize> Rect<RANK> {
    #[must_use]
    pub fn add_offset(mut self, offset: Offset<RANK>) -> Self {
        for i in 0..RANK {
            self.pos.dim[i] += offset.dim[i]
        }
        self
    }
}

/// Computes layout for a tree of nodes.
///
/// Uses a two-pass algorithm:
/// 1. `compute_size_and_rects`: Depth-first traversal that computes sizes and pushes
///    relative rects in post-order (children before parents, left before right).
/// 2. `position`: Consumes rects via `.pop()` in reverse order while traversing children
///    in reverse, creating perfect matching between stored rects and nodes.
pub fn layout<const RANK: usize>(node: &mut dyn LayoutNode<RANK>) {
    let mut relative_rects = Vec::new();
    let size = compute_size_and_rects(node, &mut relative_rects);
    let rect = Rect::new(Offset::ZERO, size);
    position(node, rect, &mut relative_rects);
}

/// Computes sizes of all nodes and builds rects vector in post-order.
///
/// Performs depth-first traversal, pushing rects AFTER recursing into children.
/// This creates post-order: for each container, all descendant rects are pushed
/// before the container's own child rects.
///
/// Returns the size of the node.
fn compute_size_and_rects<const RANK: usize>(
    node: &mut dyn LayoutNode<RANK>,
    rects: &mut Vec<Rect<RANK>>,
) -> Size<RANK> {
    match node.layout_info() {
        LayoutInfo::Container {
            layout_axis,
            child_count,
        } => {
            let mut size = Size::ZERO;
            let mut offset = Offset::ZERO;

            // Recursively compute child sizes and push rects immediately
            for i in 0..child_count {
                let child = node.get_child_mut(i);
                let child_size = compute_size_and_rects(child, rects);

                rects.push(Rect::new(offset, child_size));
                offset.dim[*layout_axis] += child_size.dim[*layout_axis];

                for j in 0..RANK {
                    if j == *layout_axis {
                        size.dim[j] += child_size.dim[j];
                    } else {
                        size.dim[j] = max(size.dim[j], child_size.dim[j]);
                    }
                }
            }

            size
        }
        LayoutInfo::Leaf { size } => size,
    }
}

/// Absolutely position this node and its children.
///
/// Processes children in REVERSE order (last to first) while consuming rects via `.pop()`.
/// Since rects were pushed in post-order, popping gives us exactly the right rect for
/// each child as we traverse backwards.
fn position<const RANK: usize>(
    node: &mut dyn LayoutNode<RANK>,
    absolute_rect: Rect<RANK>,
    child_rects: &mut Vec<Rect<RANK>>,
) {
    node.set_rect(absolute_rect);

    match node.layout_info() {
        LayoutInfo::Container { child_count, .. } => {
            // Process children in backward order
            for i in (0..child_count).rev() {
                let child_relative_rect = child_rects
                    .pop()
                    .expect("Internal error: children rects do not match");
                let child_absolute_rect = child_relative_rect.add_offset(absolute_rect.pos);
                let child = node.get_child_mut(i);
                position(child, child_absolute_rect, child_rects);
            }
        }
        LayoutInfo::Leaf { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    enum TestNode {
        Container {
            layout_axis: LayoutAxis,
            children: Vec<TestNode>,
            rect: Option<Rect<2>>,
        },
        Leaf {
            size: Size<2>,
            rect: Option<Rect<2>>,
        },
    }

    impl TestNode {
        fn rect(&self) -> Rect<2> {
            match self {
                TestNode::Container { rect, .. } => rect.expect("rect not set"),
                TestNode::Leaf { rect, .. } => rect.expect("rect not set"),
            }
        }
    }

    impl LayoutNode<2> for TestNode {
        fn layout_info(&self) -> LayoutInfo<2> {
            match self {
                TestNode::Container {
                    layout_axis,
                    children,
                    ..
                } => LayoutInfo::Container {
                    layout_axis: *layout_axis,
                    child_count: children.len(),
                },
                TestNode::Leaf { size, .. } => LayoutInfo::Leaf { size: *size },
            }
        }

        fn get_child_mut(&mut self, index: usize) -> &mut dyn LayoutNode<2> {
            match self {
                TestNode::Container { children, .. } => &mut children[index],
                TestNode::Leaf { .. } => panic!("Leaf nodes have no children"),
            }
        }

        fn set_rect(&mut self, rect: Rect<2>) {
            match self {
                TestNode::Container { rect: r, .. } => *r = Some(rect),
                TestNode::Leaf { rect: r, .. } => *r = Some(rect),
            }
        }
    }

    #[test]
    fn single_leaf() {
        let mut node = leaf(100, 50);
        layout(&mut node);

        assert_eq!(node.rect(), rect(0, 0, 100, 50));
    }

    #[test]
    fn horizontal_container_three_leaves() {
        let mut node = container(0, vec![leaf(10, 20), leaf(15, 20), leaf(25, 20)]);
        layout(&mut node);

        assert_eq!(node.rect(), rect(0, 0, 50, 20));

        if let TestNode::Container { children, .. } = &node {
            assert_eq!(children[0].rect(), rect(0, 0, 10, 20));
            assert_eq!(children[1].rect(), rect(10, 0, 15, 20));
            assert_eq!(children[2].rect(), rect(25, 0, 25, 20));
        }
    }

    #[test]
    fn vertical_container_varying_widths() {
        let mut node = container(1, vec![leaf(10, 20), leaf(30, 15), leaf(20, 25)]);
        layout(&mut node);

        assert_eq!(node.rect(), rect(0, 0, 30, 60));

        if let TestNode::Container { children, .. } = &node {
            assert_eq!(children[0].rect(), rect(0, 0, 10, 20));
            assert_eq!(children[1].rect(), rect(0, 20, 30, 15));
            assert_eq!(children[2].rect(), rect(0, 35, 20, 25));
        }
    }

    #[test]
    fn nested_containers() {
        let mut node = container(
            1,
            vec![
                container(0, vec![leaf(10, 20), leaf(15, 20)]),
                container(0, vec![leaf(20, 30), leaf(25, 30)]),
            ],
        );
        layout(&mut node);

        assert_eq!(node.rect(), rect(0, 0, 45, 50));

        if let TestNode::Container { children, .. } = &node {
            assert_eq!(children[0].rect(), rect(0, 0, 25, 20));
            assert_eq!(children[1].rect(), rect(0, 20, 45, 30));

            if let TestNode::Container {
                children: nested, ..
            } = &children[0]
            {
                assert_eq!(nested[0].rect(), rect(0, 0, 10, 20));
                assert_eq!(nested[1].rect(), rect(10, 0, 15, 20));
            }

            if let TestNode::Container {
                children: nested, ..
            } = &children[1]
            {
                assert_eq!(nested[0].rect(), rect(0, 20, 20, 30));
                assert_eq!(nested[1].rect(), rect(20, 20, 25, 30));
            }
        }
    }

    #[test]
    fn empty_container() {
        let mut node = container(0, vec![]);
        layout(&mut node);

        assert_eq!(node.rect(), rect(0, 0, 0, 0));
    }

    fn leaf(width: u32, height: u32) -> TestNode {
        TestNode::Leaf {
            size: Size {
                dim: [width, height],
            },
            rect: None,
        }
    }

    fn container(axis: usize, children: Vec<TestNode>) -> TestNode {
        TestNode::Container {
            layout_axis: LayoutAxis(axis),
            children,
            rect: None,
        }
    }

    fn offset(x: u32, y: u32) -> Offset<2> {
        Offset { dim: [x, y] }
    }

    fn size(w: u32, h: u32) -> Size<2> {
        Size { dim: [w, h] }
    }

    fn rect(x: u32, y: u32, w: u32, h: u32) -> Rect<2> {
        Rect::new(offset(x, y), size(w, h))
    }
}
