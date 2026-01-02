use std::cmp::max;

use derive_more::{Deref, From, Into};

mod dimensional;
use dimensional::{DimensionalOffset, DimensionalRect, DimensionalSize};

#[derive(Debug, Copy, Clone, From, Into, Deref)]
pub struct LayoutAxis(usize);

pub enum LayoutInfo<S: DimensionalSize> {
    Container {
        layout_axis: LayoutAxis,
        child_count: usize,
    },
    Leaf {
        size: S,
    },
}

pub trait LayoutNode {
    type Rect: DimensionalRect;

    fn layout_info(&self) -> LayoutInfo<<Self::Rect as DimensionalRect>::Size>;
    fn get_child_mut(&mut self, _index: usize) -> &mut dyn LayoutNode<Rect = Self::Rect> {
        panic!("This LayoutNode is a container, but can't access its children!")
    }
    fn set_rect(&mut self, rect: Self::Rect);
}

/// Computes layout for a tree of nodes.
///
/// Uses a two-pass algorithm:
/// 1. `compute_size_and_rects`: Depth-first traversal that computes sizes and pushes
///    relative rects in post-order (children before parents, left before right).
/// 2. `position`: Consumes rects via `.pop()` in reverse order while traversing children
///    in reverse, creating perfect matching between stored rects and nodes.
pub fn layout<N: LayoutNode>(node: &mut N)
where
    N::Rect: DimensionalRect,
{
    let mut relative_rects = Vec::new();
    let size = compute_size_and_rects(node, &mut relative_rects);
    let rect = N::Rect::from_offset_size(<N::Rect as DimensionalRect>::Offset::zero(), size);
    position(node, rect, &mut relative_rects);
}

/// Computes sizes of all nodes and builds rects vector in post-order.
///
/// Performs depth-first traversal, pushing rects AFTER recursing into children.
/// This creates post-order: for each container, all descendant rects are pushed
/// before the container's own child rects.
///
/// Returns the size of the node.
fn compute_size_and_rects<R: DimensionalRect>(
    node: &mut dyn LayoutNode<Rect = R>,
    rects: &mut Vec<R>,
) -> R::Size {
    match node.layout_info() {
        LayoutInfo::Container {
            layout_axis,
            child_count,
        } => {
            let mut size = R::Size::empty();
            let mut offset = R::Offset::zero();
            let rank = R::Size::RANK;

            // Recursively compute child sizes and push rects immediately
            for i in 0..child_count {
                let child = node.get_child_mut(i);
                let child_size = compute_size_and_rects(child, rects);

                rects.push(R::from_offset_size(offset, child_size));

                let child_size_at_axis = child_size.get(*layout_axis) as i32;
                let current_offset = offset.get(*layout_axis);
                offset.set(*layout_axis, current_offset + child_size_at_axis);

                for j in 0..rank {
                    if j == *layout_axis {
                        let current = size.get(j);
                        let child_val = child_size.get(j);
                        size.set(j, current + child_val);
                    } else {
                        let current = size.get(j);
                        let child_val = child_size.get(j);
                        size.set(j, max(current, child_val));
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
fn position<R: DimensionalRect>(
    node: &mut dyn LayoutNode<Rect = R>,
    absolute_rect: R,
    child_rects: &mut Vec<R>,
) {
    node.set_rect(absolute_rect);

    match node.layout_info() {
        LayoutInfo::Container { child_count, .. } => {
            // Process children in backward order
            for i in (0..child_count).rev() {
                let child_relative_rect = child_rects
                    .pop()
                    .expect("Internal error: children rects do not match");
                let child_absolute_rect = add_offset_to_rect(child_relative_rect, absolute_rect);
                let child = node.get_child_mut(i);
                position(child, child_absolute_rect, child_rects);
            }
        }
        LayoutInfo::Leaf { .. } => {}
    }
}

fn add_offset_to_rect<R: DimensionalRect>(mut rect: R, offset_rect: R) -> R {
    let rank = R::RANK;
    for i in 0..rank {
        let pos = rect.get(i);
        let offset = offset_rect.get(i);
        rect.set(i, pos + offset);
    }
    rect
}

#[cfg(test)]
mod tests {
    use massive_geometry::{RectPx, SizePx};

    use super::*;

    enum TestNode {
        Container {
            layout_axis: LayoutAxis,
            children: Vec<TestNode>,
            rect: Option<RectPx>,
        },
        Leaf {
            size: SizePx,
            rect: Option<RectPx>,
        },
    }

    impl TestNode {
        fn rect(&self) -> RectPx {
            match self {
                TestNode::Container { rect, .. } => rect.expect("rect not set"),
                TestNode::Leaf { rect, .. } => rect.expect("rect not set"),
            }
        }
    }

    impl LayoutNode for TestNode {
        type Rect = RectPx;

        fn layout_info(&self) -> LayoutInfo<SizePx> {
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

        fn get_child_mut(&mut self, index: usize) -> &mut dyn LayoutNode<Rect = Self::Rect> {
            match self {
                TestNode::Container { children, .. } => &mut children[index],
                TestNode::Leaf { .. } => panic!("Leaf nodes have no children"),
            }
        }

        fn set_rect(&mut self, rect: RectPx) {
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
            size: SizePx::new(width, height),
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

    fn rect(x: i32, y: i32, w: u32, h: u32) -> RectPx {
        RectPx::new(euclid::point2(x, y), euclid::size2(w as i32, h as i32))
    }
}
