use std::{cmp::max, ops::Add};

use derive_more::{Constructor, Deref, From, Into};

#[derive(Debug, Copy, Clone, From, Into, Deref)]
pub struct LayoutAxis(usize);

pub enum NodeMeta<CI, const RANK: usize> {
    Container {
        layout_axis: LayoutAxis,
        children: CI,
    },
    Node {
        size: Size<RANK>,
    },
}

pub trait LayoutNode<const RANK: usize>: Sized {
    type ChildIter<'c>: Iterator<Item = &'c Self>
    where
        Self: 'c;

    type ChildIterMut<'c>: Iterator<Item = &'c mut Self> + DoubleEndedIterator
    where
        Self: 'c;

    /// If this is a container, returns container options and child iterator, and container options.
    fn meta(&self) -> NodeMeta<Self::ChildIter<'_>, RANK>;
    fn meta_mut(&mut self) -> NodeMeta<Self::ChildIterMut<'_>, RANK>;

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

pub fn layout<N, const RANK: usize>(node: &mut N)
where
    N: LayoutNode<RANK>,
{
    let mut relative_rects = Vec::new();
    let size = compute_size(node, &mut relative_rects);
    let rect = Rect::new(Offset::ZERO, size);
    let outer_rect = position(node, rect, &mut relative_rects);
    node.set_rect(outer_rect);
}

/// Computes sizes of all nodes and returns their container relative offsets in the order traversed.
///
/// The resulting rects are in pre-order (depth first) traversal with children processed in order.
fn compute_size<N, const RANK: usize>(node: &N, child_rects: &mut Vec<Rect<RANK>>) -> Size<RANK>
where
    N: LayoutNode<RANK>,
{
    match node.meta() {
        NodeMeta::Container {
            layout_axis,
            children,
        } => {
            let mut size = Size::ZERO;
            let mut offset = Offset::<RANK>::ZERO;
            for node in children {
                let c_size = compute_size(node, child_rects);
                for i in 0..RANK {
                    if i == *layout_axis {
                        size.dim[i] += c_size.dim[i];
                    } else {
                        size.dim[i] = max(size.dim[i], c_size.dim[i]);
                    }
                }

                child_rects.push(Rect::new(offset, c_size));

                // Adjust offset
                offset.dim[*layout_axis] += c_size.dim[*layout_axis]
                // in all other axis, offset stays 0
            }
            size
        }
        NodeMeta::Node { size } => size,
    }
}

/// Absolutely position children.
///
/// Architecture: This needs ChildIterMut and meta_mut(). Is there a way around this?
fn position<N, const RANK: usize>(
    node: &mut N,
    absolute_rect: Rect<RANK>,
    child_rects: &mut Vec<Rect<RANK>>,
) -> Rect<RANK>
where
    N: LayoutNode<RANK>,
{
    match node.meta_mut() {
        NodeMeta::Container { children, .. } => {
            // Process children in reverse to keep in sync with the post-order traversal.
            for node in children.rev() {
                let child_relative_rect = child_rects.pop().unwrap();
                let child_absolute_rect = child_relative_rect.add_offset(absolute_rect.pos);
                let positioned = position(node, child_absolute_rect, child_rects);
                node.set_rect(positioned);
            }
        }
        NodeMeta::Node { .. } => {}
    }

    absolute_rect
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
        type ChildIter<'c> = std::slice::Iter<'c, TestNode>;
        type ChildIterMut<'c> = std::slice::IterMut<'c, TestNode>;

        fn meta(&self) -> NodeMeta<Self::ChildIter<'_>, 2> {
            match self {
                TestNode::Container {
                    layout_axis,
                    children,
                    ..
                } => NodeMeta::Container {
                    layout_axis: *layout_axis,
                    children: children.iter(),
                },
                TestNode::Leaf { size, .. } => NodeMeta::Node { size: *size },
            }
        }

        fn meta_mut(&mut self) -> NodeMeta<Self::ChildIterMut<'_>, 2> {
            match self {
                TestNode::Container {
                    layout_axis,
                    children,
                    ..
                } => NodeMeta::Container {
                    layout_axis: *layout_axis,
                    children: children.iter_mut(),
                },
                TestNode::Leaf { size, .. } => NodeMeta::Node { size: *size },
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
