//! Third attempt at the layout engine.
//!
//! First: was a trait / node based layouter.
//!
//! Second: This is a non-trait based layouter that supports providing an id, and spits out a number
//! of absolute `(Id,Rect)` pairs.
//!
//! Third: Functional variant, the main reason was that we needed to map Ids and the second one was
//! using a shared strace for memory optimization that prevented that or made it too complicated.

use std::cmp::max;

use derive_more::{From, Into};

use crate::{
    LayoutAxis,
    dimensional_types::{Box, Offset, Size, Thickness},
};

pub fn leaf<Id: Clone, const RANK: usize>(
    id: impl Into<Option<Id>>,
    size: impl Into<Size<RANK>>,
) -> Layout<Id, RANK> {
    Layout {
        id: id.into(),
        container: None,
        offset: Offset::ZERO,
        size: size.into(),
    }
}

pub fn container<Id: Clone, const RANK: usize>(
    id: impl Into<Option<Id>>,
    layout_axis: LayoutAxis,
) -> ContainerBuilder<Id, RANK> {
    ContainerBuilder {
        id: id.into(),
        container: Container {
            layout_axis,
            padding: Thickness::ZERO,
            spacing: 0,
            children: Vec::new(),
        },
    }
}

#[derive(Debug)]
pub struct ContainerBuilder<Id: Clone, const RANK: usize> {
    id: Option<Id>,
    container: Container<Id, RANK>,
}

impl<Id: Clone, const RANK: usize> ContainerBuilder<Id, RANK> {
    pub fn padding(
        mut self,
        leading: impl Into<Padding<RANK>>,
        trailing: impl Into<Padding<RANK>>,
    ) -> Self {
        self.container.padding = Thickness {
            leading: leading.into().into(),
            trailing: trailing.into().into(),
        };
        self
    }

    pub fn spacing(mut self, spacing: u32) -> Self {
        self.container.spacing = spacing;
        self
    }

    pub fn child(&mut self, child: Layout<Id, RANK>) {
        self.container.children.push(child);
    }

    pub fn with_child(mut self, child: Layout<Id, RANK>) -> Self {
        self.container.children.push(child);
        self
    }

    pub fn layout(mut self) -> Layout<Id, RANK> {
        let axis = *self.container.layout_axis;
        let mut size = Size::EMPTY;
        let mut offset: Offset<RANK> = self.container.padding.leading.into();

        // Position children and compute container size
        for (i, child) in self.container.children.iter_mut().enumerate() {
            let child_outer_size = child.outer_size();

            // Add spacing before this child (except for the first)
            if i > 0 {
                offset[axis] += self.container.spacing as i32;
            }

            // Set child's offset relative to this container's content area
            child.offset = offset;

            // Advance offset along layout axis
            offset[axis] += child_outer_size[axis] as i32;

            // Accumulate size: sum along axis, max perpendicular
            for dim in 0..RANK {
                if dim == axis {
                    size[dim] += child_outer_size[dim];
                    if i > 0 {
                        size[dim] += self.container.spacing;
                    }
                } else {
                    size[dim] = max(size[dim], child_outer_size[dim]);
                }
            }
        }

        Layout {
            id: self.id,
            container: Some(self.container),
            offset: Offset::ZERO,
            size,
        }
    }
}

#[derive(Debug)]
pub struct Layout<Id: Clone, const RANK: usize> {
    // May also be None for leaves (imagine empty spacing)
    id: Option<Id>,

    container: Option<Container<Id, RANK>>,

    // Architecture: Replace the following two by a `Rect<RANK>`?

    // The placement offset of this layout (the parent relative offset)
    // 0,0). If this is a container, this does include leading padding.
    offset: Offset<RANK>,

    // The inner size of this. Does not include padding. Does include spacing.
    size: Size<RANK>,
}

#[derive(Debug)]
struct Container<Id: Clone, const RANK: usize> {
    layout_axis: LayoutAxis,

    padding: Thickness<RANK>,
    spacing: u32,
    children: Vec<Layout<Id, RANK>>,
}

impl<Id: Clone, const RANK: usize> Container<Id, RANK> {
    fn map_id<NewId: Clone>(self, f: &impl Fn(Id) -> NewId) -> Container<NewId, RANK> {
        Container {
            layout_axis: self.layout_axis,
            padding: self.padding,
            spacing: self.spacing,
            children: self
                .children
                .into_iter()
                .map(|child| child.map_id_ref(f))
                .collect(),
        }
    }
}

impl<Id: Clone, const RANK: usize> Layout<Id, RANK> {
    pub fn outer_size(&self) -> Size<RANK> {
        if let Some(ref container) = self.container {
            container.padding.leading + self.size + container.padding.trailing
        } else {
            self.size
        }
    }

    pub fn with_id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }

    pub fn map_id<NewId: Clone>(self, f: impl Fn(Id) -> NewId) -> Layout<NewId, RANK> {
        // Need to use a reference here to be able to call it multiple times.
        // Alternative is to require Clone.
        self.map_id_ref(&f)
    }

    fn map_id_ref<NewId: Clone>(self, f: &impl Fn(Id) -> NewId) -> Layout<NewId, RANK> {
        Layout {
            id: self.id.map(f),
            container: self.container.map(|c| c.map_id(f)),
            offset: self.offset,
            size: self.size,
        }
    }

    pub fn place<BX>(self, absolute_offset: impl Into<Offset<RANK>>) -> Vec<(Id, BX)>
    where
        BX: From<Box<RANK>>,
    {
        let mut vec = Vec::new();
        self.place_inline(absolute_offset, |id, r| vec.push((id, r)));
        vec
    }

    pub fn place_inline<BX>(
        self,
        absolute_offset: impl Into<Offset<RANK>>,
        mut set_rect: impl FnMut(Id, BX),
    ) where
        BX: From<Box<RANK>>,
    {
        let absolute_offset: Offset<RANK> = absolute_offset.into();
        self.place_rec(absolute_offset, &mut |id, bx| set_rect(id, bx.into()));
    }

    fn place_rec(self, absolute_offset: Offset<RANK>, out: &mut impl FnMut(Id, Box<RANK>)) {
        // Compute absolute position of this layout
        let abs_offset = absolute_offset + self.offset;

        let outer_size = self.outer_size();
        let id = self.id;
        let container = self.container;

        // Emit this layout's box if it has an id
        if let Some(id) = id {
            out(id, Box::new(abs_offset, outer_size));
        }

        // Recursively place children with accumulated offset
        if let Some(container) = container {
            for child in container.children {
                child.place_rec(abs_offset, out);
            }
        }
    }
}

/// Convenience type to convert from u32, (u32,u32) to a padding value.

#[derive(Debug, Clone, Copy, PartialEq, Eq, From, Into)]
pub struct Padding<const RANK: usize>(Size<RANK>);

impl<const RANK: usize> From<u32> for Padding<RANK> {
    fn from(value: u32) -> Self {
        [value; RANK].into()
    }
}

impl<const RANK: usize> From<[u32; RANK]> for Padding<RANK> {
    fn from(value: [u32; RANK]) -> Self {
        Size::from(value).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_leaf() {
        let mut root = container(0, LayoutAxis::HORIZONTAL);
        root.child(leaf(1, size(100, 50)));
        let results = root.layout().place(point(0, 0));

        assert_eq!(results.len(), 2);
        assert_eq!(results[0], (0, rect(0, 0, 100, 50)));
        assert_eq!(results[1], (1, rect(0, 0, 100, 50)));
    }

    #[test]
    fn horizontal_container_with_leaves() {
        let mut root = container(0, LayoutAxis::HORIZONTAL);
        root.child(leaf(1, size(100, 50)));
        root.child(leaf(2, size(200, 30)));
        root.child(leaf(3, size(150, 40)));
        let results = root.layout().place(point(0, 0));

        assert_eq!(results.len(), 4);
        // Root should have width = sum, height = max
        assert_eq!(results[0], (0, rect(0, 0, 450, 50)));
        // Children laid out horizontally in forward order (1, 2, 3)
        assert_eq!(results[1], (1, rect(0, 0, 100, 50)));
        assert_eq!(results[2], (2, rect(100, 0, 200, 30)));
        assert_eq!(results[3], (3, rect(300, 0, 150, 40)));
    }

    #[test]
    fn vertical_container() {
        let mut root = container(0, LayoutAxis::VERTICAL);
        root.child(leaf(1, size(100, 50)));
        root.child(leaf(2, size(200, 30)));
        root.child(leaf(3, size(150, 40)));
        let results = root.layout().place(point(0, 0));

        assert_eq!(results.len(), 4);
        // Root should have width = max, height = sum
        assert_eq!(results[0], (0, rect(0, 0, 200, 120)));
        // Children laid out vertically in forward order (1, 2, 3)
        assert_eq!(results[1], (1, rect(0, 0, 100, 50)));
        assert_eq!(results[2], (2, rect(0, 50, 200, 30)));
        assert_eq!(results[3], (3, rect(0, 80, 150, 40)));
    }

    #[test]
    fn empty_container() {
        let root = container(0, LayoutAxis::HORIZONTAL);
        let results = root.layout().place(point(0, 0));

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], (0, rect(0, 0, 0, 0)));
    }

    #[test]
    fn custom_offset() {
        let mut root = container(0, LayoutAxis::HORIZONTAL);
        root.child(leaf(1, size(10, 10)));
        root.child(leaf(2, size(20, 20)));
        let results = root.layout().place(point(100, 200));

        assert_eq!(results.len(), 3);
        // All positions should be offset by (100, 200), children in forward order
        assert_eq!(results[0], (0, rect(100, 200, 30, 20)));
        assert_eq!(results[1], (1, rect(100, 200, 10, 10)));
        assert_eq!(results[2], (2, rect(110, 200, 20, 20)));
    }

    #[test]
    fn size_accumulation_along_axis() {
        let mut root = container(0, LayoutAxis::HORIZONTAL);
        root.child(leaf(1, size(10, 50)));
        root.child(leaf(2, size(20, 30)));
        root.child(leaf(3, size(30, 40)));
        let results = root.layout().place(point(0, 0));

        // Along horizontal axis: widths sum (10+20+30=60)
        // Perpendicular (vertical): heights max (50)
        assert_eq!(results[0], (0, rect(0, 0, 60, 50)));
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn depth_axis_on_2d_rect_panics() {
        // Test that using DEPTH axis (index 2) on 2D rects properly panics
        // since RANK is 2
        let mut root = container(0, LayoutAxis::DEPTH);
        root.child(leaf(1, size(100, 200))); // This should panic when accessing index 2
        let _: Vec<(usize, Box<2>)> = root.layout().place(point(0, 0));
    }

    // This test demonstrates the builder pattern with nested containers
    #[test]
    fn nested_container_with_siblings() {
        let mut root = container(0, LayoutAxis::HORIZONTAL);

        root.child(leaf(1, size(50, 50)));

        {
            let mut nested = container(2, LayoutAxis::VERTICAL);
            nested.child(leaf(3, size(20, 30)));
            nested.child(leaf(4, size(25, 35)));
            root.child(nested.layout());
        }

        root.child(leaf(5, size(60, 60)));

        let results = root.layout().place(point(0, 0));

        // Root contains: leaf(1) 50x50, container(2) 25x65, leaf(5) 60x60
        // Container(2) contains: leaf(3) 20x30, leaf(4) 25x35 (vertical: width=max(20,25)=25, height=30+35=65)
        // Root horizontal: width=50+25+60=135, height=max(50,65,60)=65
        assert_eq!(results.len(), 6);
        assert_eq!(results[0], (0, rect(0, 0, 135, 65)));
        // Children in forward order: 1, 2 (with nested 3, 4), 5
        assert_eq!(results[1], (1, rect(0, 0, 50, 50)));
        assert_eq!(results[2], (2, rect(50, 0, 25, 65)));
        assert_eq!(results[3], (3, rect(50, 0, 20, 30)));
        assert_eq!(results[4], (4, rect(50, 30, 25, 35)));
        assert_eq!(results[5], (5, rect(75, 0, 60, 60)));
    }

    // Helper to create Rect<2>
    fn rect(x: i32, y: i32, w: u32, h: u32) -> Box<2> {
        Box::new(point(x, y).into(), size(w, h).into())
    }

    // Helper to create Size<2>
    fn size(w: u32, h: u32) -> [u32; 2] {
        [w, h]
    }

    // Helper to create Offset<2>
    fn point(x: i32, y: i32) -> [i32; 2] {
        [x, y]
    }

    #[test]
    fn horizontal_container_with_padding() {
        let mut root = container(0, LayoutAxis::HORIZONTAL).padding(size(10, 20), size(30, 40));
        root.child(leaf(1, size(100, 50)));
        root.child(leaf(2, size(200, 30)));
        let results = root.layout().place(point(0, 0));

        assert_eq!(results.len(), 3);
        // Outer size: padding.leading + inner + padding.trailing
        // Width: 10 + (100 + 200) + 30 = 340
        // Height: 20 + max(50, 30) + 40 = 110
        assert_eq!(results[0], (0, rect(0, 0, 340, 110)));
        // Children offset by leading padding (10, 20), laid out in forward order
        assert_eq!(results[1], (1, rect(10, 20, 100, 50)));
        assert_eq!(results[2], (2, rect(110, 20, 200, 30)));
    }

    #[test]
    fn vertical_container_with_padding() {
        let mut root = container(0, LayoutAxis::VERTICAL).padding(size(5, 10), size(15, 20));
        root.child(leaf(1, size(100, 50)));
        root.child(leaf(2, size(200, 30)));
        let results = root.layout().place(point(0, 0));

        assert_eq!(results.len(), 3);
        // Outer size:
        // Width: 5 + max(100, 200) + 15 = 220
        // Height: 10 + (50 + 30) + 20 = 110
        assert_eq!(results[0], (0, rect(0, 0, 220, 110)));
        // Children offset by leading padding (5, 10), laid out vertically in forward order
        assert_eq!(results[1], (1, rect(5, 10, 100, 50)));
        assert_eq!(results[2], (2, rect(5, 60, 200, 30)));
    }

    #[test]
    fn nested_container_with_padding() {
        let mut root = container(0, LayoutAxis::HORIZONTAL).padding(size(10, 10), size(10, 10));

        root.child(leaf(1, size(50, 50)));

        {
            let mut nested = container(2, LayoutAxis::VERTICAL).padding(size(5, 5), size(5, 5));
            nested.child(leaf(3, size(20, 30)));
            nested.child(leaf(4, size(25, 35)));
            // Container inner size: width=max(20,25)=25, height=30+35=65
            // Container outer size: width=5+25+5=35, height=5+65+5=75
            root.child(nested.layout());
        }

        root.child(leaf(5, size(60, 60)));

        let results = root.layout().place(point(0, 0));

        // Root contains: leaf(1) 50x50, container(2) 35x75, leaf(5) 60x60
        // Root inner: width=50+35+60=145, height=max(50,75,60)=75
        // Root outer: width=10+145+10=165, height=10+75+10=95
        assert_eq!(results.len(), 6);
        assert_eq!(results[0], (0, rect(0, 0, 165, 95)));
        // Root children offset by (10, 10), in forward order: 1, 2, 5
        assert_eq!(results[1], (1, rect(10, 10, 50, 50)));
        assert_eq!(results[2], (2, rect(60, 10, 35, 75)));
        // Container(2) children offset by container's abs position (60, 10) + padding (5, 5)
        // Leaf 3 at relative (5, 5), absolute (65, 15)
        // Leaf 4 at relative (5, 35), absolute (65, 45)
        assert_eq!(results[3], (3, rect(65, 15, 20, 30)));
        assert_eq!(results[4], (4, rect(65, 45, 25, 35)));
        assert_eq!(results[5], (5, rect(95, 10, 60, 60)));
    }

    #[test]
    fn padding_with_empty_container() {
        let root = container(0, LayoutAxis::HORIZONTAL).padding(size(10, 20), size(30, 40));
        let results = root.layout().place(point(0, 0));

        assert_eq!(results.len(), 1);
        // Only padding: width=10+0+30=40, height=20+0+40=60
        assert_eq!(results[0], (0, rect(0, 0, 40, 60)));
    }

    #[test]
    fn zero_padding() {
        let mut root = container(0, LayoutAxis::HORIZONTAL).padding(size(0, 0), size(0, 0));
        root.child(leaf(1, size(100, 50)));
        let results = root.layout().place(point(0, 0));

        assert_eq!(results.len(), 2);
        // Should behave same as no padding
        assert_eq!(results[0], (0, rect(0, 0, 100, 50)));
        assert_eq!(results[1], (1, rect(0, 0, 100, 50)));
    }

    #[test]
    fn asymmetric_padding() {
        let mut root = container(0, LayoutAxis::VERTICAL).padding(size(0, 10), size(20, 0));
        root.child(leaf(1, size(100, 50)));
        let results = root.layout().place(point(0, 0));

        assert_eq!(results.len(), 2);
        // Width: 0 + 100 + 20 = 120
        // Height: 10 + 50 + 0 = 60
        assert_eq!(results[0], (0, rect(0, 0, 120, 60)));
        // Child offset by leading padding (0, 10)
        assert_eq!(results[1], (1, rect(0, 10, 100, 50)));
    }

    #[test]
    fn horizontal_container_with_spacing() {
        let mut root = container(0, LayoutAxis::HORIZONTAL).spacing(10);
        root.child(leaf(1, size(100, 50)));
        root.child(leaf(2, size(80, 60)));
        root.child(leaf(3, size(120, 40)));
        let results = root.layout().place(point(0, 0));

        assert_eq!(results.len(), 4);
        // Width: 100 + 10 + 80 + 10 + 120 = 320
        // Height: max(50, 60, 40) = 60
        assert_eq!(results[0], (0, rect(0, 0, 320, 60)));
        // Children in forward order: 1, 2, 3
        assert_eq!(results[1], (1, rect(0, 0, 100, 50)));
        assert_eq!(results[2], (2, rect(110, 0, 80, 60)));
        assert_eq!(results[3], (3, rect(200, 0, 120, 40)));
    }

    #[test]
    fn spacing_with_padding() {
        let mut root = container(0, LayoutAxis::HORIZONTAL)
            .padding(size(5, 3), size(7, 4))
            .spacing(10);
        root.child(leaf(1, size(100, 50)));
        root.child(leaf(2, size(80, 60)));
        let results = root.layout().place(point(0, 0));

        assert_eq!(results.len(), 3);
        // Width: 5 + 100 + 10 + 80 + 7 = 202
        // Height: 3 + max(50, 60) + 4 = 67
        assert_eq!(results[0], (0, rect(0, 0, 202, 67)));
        // Children in forward order: 1, 2
        // First child offset by leading padding
        assert_eq!(results[1], (1, rect(5, 3, 100, 50)));
        // Second child: 5 (leading) + 100 (first) + 10 (spacing)
        assert_eq!(results[2], (2, rect(115, 3, 80, 60)));
    }
}
