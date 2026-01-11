//! Second attempt at the layout engine.
//!
//! This is a non-trait based layouter that supports providing an id, and spits out a number of
//! absolute `(Id,Rect)` pairs.

use std::{cmp::max, mem};

use crate::{
    LayoutAxis,
    dimensional_types::{Box, Offset, Size, Thickness},
};

#[derive(Debug)]
pub struct Layouter<'a, Id: Clone, const RANK: usize> {
    id: Id,
    parent: Option<&'a mut Inner<Id, RANK>>,
    inner: Inner<Id, RANK>,
}

#[derive(Debug)]
struct Inner<Id: Clone, const RANK: usize> {
    trace: Vec<TraceEntry<Id, RANK>>,
    layout_axis: LayoutAxis,

    padding: Thickness<RANK>,
    spacing: u32,

    // The placement offset of the children (not the parent relative offset, which is always
    // 0,0). It includes leading padding.
    offset: Offset<RANK>,

    // The inner size of the children. Does not include padding.
    size: Size<RANK>,

    children: usize,
}

#[derive(Debug)]
struct TraceEntry<Id, const RANK: usize> {
    id: Id,
    bx: Box<RANK>,
    /// The number of children, 0 if this is a leaf.
    children: usize,
}

impl<Id: Clone, const RANK: usize> Drop for Layouter<'_, Id, RANK> {
    fn drop(&mut self) {
        if let Some(parent) = self.parent.take() {
            parent.trace = mem::take(&mut self.inner.trace);
            parent.child(self.id.clone(), self.outer_size(), self.inner.children);
        }
    }
}

pub type BoxComponents<const RANK: usize> = ([i32; RANK], [u32; RANK]);

impl<const RANK: usize> From<Box<RANK>> for BoxComponents<RANK> {
    fn from(value: Box<RANK>) -> Self {
        (value.offset.0, value.size.0)
    }
}

impl<'a, Id: Clone, const RANK: usize> Layouter<'a, Id, RANK> {
    pub fn root(id: Id, layout_axis: LayoutAxis) -> Self {
        Self::new(None, id, layout_axis)
    }

    fn new(mut parent: Option<&'a mut Inner<Id, RANK>>, id: Id, layout_axis: LayoutAxis) -> Self {
        // If there is a parent, get the trace.
        let trace = parent
            .as_mut()
            .map_or_else(Vec::new, |parent| mem::take(&mut parent.trace));

        Self {
            id,
            parent,
            inner: Inner {
                trace,
                layout_axis,
                padding: Thickness::ZERO,
                spacing: 0,
                offset: Offset::ZERO,
                size: Size::EMPTY,
                children: 0,
            },
        }
    }

    pub fn leaf(&mut self, id: Id, child_size: impl Into<[u32; RANK]>) {
        self.inner.child(id, child_size.into().into(), 0);
    }

    pub fn container<'b>(&'b mut self, id: Id, layout_axis: LayoutAxis) -> Layouter<'b, Id, RANK> {
        Layouter::new(Some(&mut self.inner), id, layout_axis)
    }

    pub fn padding(
        mut self,
        leading: impl Into<[u32; RANK]>,
        trailing: impl Into<[u32; RANK]>,
    ) -> Self {
        let inner = &mut self.inner;
        if inner.children > 0 {
            panic!("padding() must be called before adding any children");
        }

        inner.padding = Thickness {
            leading: leading.into().into(),
            trailing: trailing.into().into(),
        };
        // Update offset to account for leading padding
        for i in 0..RANK {
            inner.offset[i] = inner.padding.leading[i] as i32;
        }
        self
    }

    pub fn spacing(mut self, spacing: u32) -> Self {
        let inner = &mut self.inner;
        if inner.children > 0 {
            panic!("spacing() must be called before adding any children");
        }
        inner.spacing = spacing;
        self
    }

    pub fn outer_size(&self) -> Size<RANK> {
        let inner = &self.inner;
        inner.padding.leading + inner.size + inner.padding.trailing
    }

    pub fn place<BX>(self, absolute_offset: impl Into<[i32; RANK]>) -> Vec<(Id, BX)>
    where
        BX: From<BoxComponents<RANK>>,
    {
        let mut vec = Vec::new();
        self.place_inline(absolute_offset, |(id, r)| vec.push((id, r)));
        vec
    }

    pub fn place_inline<BX>(
        mut self,
        absolute_offset: impl Into<[i32; RANK]>,
        mut out: impl FnMut((Id, BX)),
    ) where
        BX: From<BoxComponents<RANK>>,
    {
        if self.parent.is_some() {
            panic!("Layout finalization can only be done on root containers");
        }

        let mut out = |(id, bx): (Id, Box<RANK>)| {
            let box_components: BoxComponents<RANK> = bx.into();
            out((id, box_components.into()))
        };

        let entry = TraceEntry {
            id: self.id.clone(),
            bx: Box::new(Offset::ZERO, self.outer_size()),
            children: self.inner.children,
        };

        place_rec(
            &mut self.inner.trace,
            absolute_offset.into().into(),
            entry,
            &mut out,
        );
    }
}

impl<Id: Clone, const RANK: usize> Inner<Id, RANK> {
    fn child(&mut self, id: Id, child_size: Size<RANK>, children: usize) {
        let axis = *self.layout_axis;

        // Add spacing before this child (except for the first child), and
        // add the spacing contribution to the container size.
        if self.children > 0 {
            self.offset[axis] += self.spacing as i32;
            self.size[axis] += self.spacing;
        }

        let child_relative_box = Box::new(self.offset, child_size);
        self.trace.push(TraceEntry {
            id,
            bx: child_relative_box,
            children,
        });

        for i in 0..RANK {
            if i == axis {
                self.size[i] += child_size[i];
            } else {
                self.size[i] = max(self.size[i], child_size[i]);
            }
        }

        self.offset[axis] += child_size[axis] as i32;

        self.children += 1;
    }
}

fn place_rec<Id, const RANK: usize>(
    trace: &mut Vec<TraceEntry<Id, RANK>>,
    offset: Offset<RANK>,
    this: TraceEntry<Id, RANK>,
    out: &mut impl FnMut((Id, Box<RANK>)),
) {
    let absolute_rect = add_offset(this.bx, offset);
    out((this.id, absolute_rect));
    // Children are already positioned relative to padding.leading in their parent,
    // so just use the absolute rect's offset
    let children_offset = absolute_rect.offset;

    for _ in 0..this.children {
        let child = trace
            .pop()
            .expect("Internal error: Trace of children does not match");
        place_rec(trace, children_offset, child, out);
    }
}

fn add_offset<const RANK: usize>(mut rect: Box<RANK>, offset: Offset<RANK>) -> Box<RANK> {
    for i in 0..RANK {
        rect.offset[i] += offset[i];
    }
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestLayout<'a> = Layouter<'a, usize, 2>;

    #[test]
    fn single_leaf() {
        let mut root = TestLayout::root(0, LayoutAxis::HORIZONTAL);
        root.leaf(1, size(100, 50));
        let results = root.place(point(0, 0));

        assert_eq!(results.len(), 2);
        assert_eq!(results[0], (0, rect(0, 0, 100, 50)));
        assert_eq!(results[1], (1, rect(0, 0, 100, 50)));
    }

    #[test]
    fn horizontal_container_with_leaves() {
        let mut root = TestLayout::root(0, LayoutAxis::HORIZONTAL);
        root.leaf(1, size(100, 50));
        root.leaf(2, size(200, 30));
        root.leaf(3, size(150, 40));
        let results = root.place(point(0, 0));

        assert_eq!(results.len(), 4);
        // Root should have width = sum, height = max
        assert_eq!(results[0], (0, rect(0, 0, 450, 50)));
        // Children laid out horizontally in reverse order (3, 2, 1)
        assert_eq!(results[1], (3, rect(300, 0, 150, 40)));
        assert_eq!(results[2], (2, rect(100, 0, 200, 30)));
        assert_eq!(results[3], (1, rect(0, 0, 100, 50)));
    }

    #[test]
    fn vertical_container() {
        let mut root = TestLayout::root(0, LayoutAxis::VERTICAL);
        root.leaf(1, size(100, 50));
        root.leaf(2, size(200, 30));
        root.leaf(3, size(150, 40));
        let results = root.place(point(0, 0));

        assert_eq!(results.len(), 4);
        // Root should have width = max, height = sum
        assert_eq!(results[0], (0, rect(0, 0, 200, 120)));
        // Children laid out vertically in reverse order (3, 2, 1)
        assert_eq!(results[1], (3, rect(0, 80, 150, 40)));
        assert_eq!(results[2], (2, rect(0, 50, 200, 30)));
        assert_eq!(results[3], (1, rect(0, 0, 100, 50)));
    }

    #[test]
    fn empty_container() {
        let root = TestLayout::root(0, LayoutAxis::HORIZONTAL);
        let results = root.place(point(0, 0));

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], (0, rect(0, 0, 0, 0)));
    }

    #[test]
    fn custom_offset() {
        let mut root = TestLayout::root(0, LayoutAxis::HORIZONTAL);
        root.leaf(1, size(10, 10));
        root.leaf(2, size(20, 20));
        let results = root.place(point(100, 200));

        assert_eq!(results.len(), 3);
        // All positions should be offset by (100, 200), children in reverse order
        assert_eq!(results[0], (0, rect(100, 200, 30, 20)));
        assert_eq!(results[1], (2, rect(110, 200, 20, 20)));
        assert_eq!(results[2], (1, rect(100, 200, 10, 10)));
    }

    #[test]
    fn size_accumulation_along_axis() {
        let mut root = TestLayout::root(0, LayoutAxis::HORIZONTAL);
        root.leaf(1, size(10, 50));
        root.leaf(2, size(20, 30));
        root.leaf(3, size(30, 40));
        let results = root.place(point(0, 0));

        // Along horizontal axis: widths sum (10+20+30=60)
        // Perpendicular (vertical): heights max (50)
        assert_eq!(results[0], (0, rect(0, 0, 60, 50)));
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn depth_axis_on_2d_rect_panics() {
        // Test that using DEPTH axis (index 2) on 2D rects properly panics
        // since RANK is 2
        let mut root = TestLayout::root(0, LayoutAxis::DEPTH);
        root.leaf(1, size(100, 200)); // This should panic when accessing index 2
    }

    // This test demonstrates the RAII pattern with LayoutInner separation
    #[test]
    fn nested_container_with_siblings() {
        let mut root = TestLayout::root(0, LayoutAxis::HORIZONTAL);

        root.leaf(1, size(50, 50));

        {
            let mut container = root.container(2, LayoutAxis::VERTICAL);
            container.leaf(3, size(20, 30));
            container.leaf(4, size(25, 35));
        }

        root.leaf(5, size(60, 60));

        let results = root.place(point(0, 0));

        // Root contains: leaf(1) 50x50, container(2) 25x65, leaf(5) 60x60
        // Container(2) contains: leaf(3) 20x30, leaf(4) 25x35 (vertical: width=max(20,25)=25, height=30+35=65)
        // Root horizontal: width=50+25+60=135, height=max(50,65,60)=65
        assert_eq!(results.len(), 6);
        assert_eq!(results[0], (0, rect(0, 0, 135, 65)));
        // Children in reverse: 5, 2, 1
        assert_eq!(results[1], (5, rect(75, 0, 60, 60)));
        assert_eq!(results[2], (2, rect(50, 0, 25, 65)));
        assert_eq!(results[3], (4, rect(50, 30, 25, 35)));
        assert_eq!(results[4], (3, rect(50, 0, 20, 30)));
        assert_eq!(results[5], (1, rect(0, 0, 50, 50)));
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
        let mut root =
            TestLayout::root(0, LayoutAxis::HORIZONTAL).padding(size(10, 20), size(30, 40));
        root.leaf(1, size(100, 50));
        root.leaf(2, size(200, 30));
        let results = root.place(point(0, 0));

        assert_eq!(results.len(), 3);
        // Outer size: padding.leading + inner + padding.trailing
        // Width: 10 + (100 + 200) + 30 = 340
        // Height: 20 + max(50, 30) + 40 = 110
        assert_eq!(results[0], (0, rect(0, 0, 340, 110)));
        // Children offset by leading padding (10, 20), laid out in reverse
        assert_eq!(results[1], (2, rect(110, 20, 200, 30)));
        assert_eq!(results[2], (1, rect(10, 20, 100, 50)));
    }

    #[test]
    fn vertical_container_with_padding() {
        let mut root = TestLayout::root(0, LayoutAxis::VERTICAL).padding(size(5, 10), size(15, 20));
        root.leaf(1, size(100, 50));
        root.leaf(2, size(200, 30));
        let results = root.place(point(0, 0));

        assert_eq!(results.len(), 3);
        // Outer size:
        // Width: 5 + max(100, 200) + 15 = 220
        // Height: 10 + (50 + 30) + 20 = 110
        assert_eq!(results[0], (0, rect(0, 0, 220, 110)));
        // Children offset by leading padding (5, 10), laid out vertically in reverse
        assert_eq!(results[1], (2, rect(5, 60, 200, 30)));
        assert_eq!(results[2], (1, rect(5, 10, 100, 50)));
    }

    #[test]
    fn nested_container_with_padding() {
        let mut root =
            TestLayout::root(0, LayoutAxis::HORIZONTAL).padding(size(10, 10), size(10, 10));

        root.leaf(1, size(50, 50));

        {
            let mut container = root
                .container(2, LayoutAxis::VERTICAL)
                .padding(size(5, 5), size(5, 5));
            container.leaf(3, size(20, 30));
            container.leaf(4, size(25, 35));
            // Container inner size: width=max(20,25)=25, height=30+35=65
            // Container outer size: width=5+25+5=35, height=5+65+5=75
        }

        root.leaf(5, size(60, 60));

        let results = root.place(point(0, 0));

        // Root contains: leaf(1) 50x50, container(2) 35x75, leaf(5) 60x60
        // Root inner: width=50+35+60=145, height=max(50,75,60)=75
        // Root outer: width=10+145+10=165, height=10+75+10=95
        assert_eq!(results.len(), 6);
        assert_eq!(results[0], (0, rect(0, 0, 165, 95)));
        // Root children offset by (10, 10), in reverse: 5, 2, 1
        // Leaf 5 at x: 10 (padding) + 50 (leaf1) + 35 (container2) = 95
        assert_eq!(results[1], (5, rect(95, 10, 60, 60)));
        assert_eq!(results[2], (2, rect(60, 10, 35, 75)));
        // Container(2) children offset by container's abs position (60, 10) + padding (5, 5)
        // Leaf 3 at relative (5, 5), absolute (65, 15)
        // Leaf 4 at relative (5, 35), absolute (65, 45)
        assert_eq!(results[3], (4, rect(65, 45, 25, 35)));
        assert_eq!(results[4], (3, rect(65, 15, 20, 30)));
        assert_eq!(results[5], (1, rect(10, 10, 50, 50)));
    }

    #[test]
    fn padding_with_empty_container() {
        let root = TestLayout::root(0, LayoutAxis::HORIZONTAL).padding(size(10, 20), size(30, 40));
        let results = root.place(point(0, 0));

        assert_eq!(results.len(), 1);
        // Only padding: width=10+0+30=40, height=20+0+40=60
        assert_eq!(results[0], (0, rect(0, 0, 40, 60)));
    }

    #[test]
    fn zero_padding() {
        let mut root = TestLayout::root(0, LayoutAxis::HORIZONTAL).padding(size(0, 0), size(0, 0));
        root.leaf(1, size(100, 50));
        let results = root.place(point(0, 0));

        assert_eq!(results.len(), 2);
        // Should behave same as no padding
        assert_eq!(results[0], (0, rect(0, 0, 100, 50)));
        assert_eq!(results[1], (1, rect(0, 0, 100, 50)));
    }

    #[test]
    fn asymmetric_padding() {
        let mut root = TestLayout::root(0, LayoutAxis::VERTICAL).padding(size(0, 10), size(20, 0));
        root.leaf(1, size(100, 50));
        let results = root.place(point(0, 0));

        assert_eq!(results.len(), 2);
        // Width: 0 + 100 + 20 = 120
        // Height: 10 + 50 + 0 = 60
        assert_eq!(results[0], (0, rect(0, 0, 120, 60)));
        // Child offset by leading padding (0, 10)
        assert_eq!(results[1], (1, rect(0, 10, 100, 50)));
    }

    #[test]
    #[should_panic(expected = "padding() must be called before adding any children")]
    fn padding_after_children_panics() {
        let mut root = TestLayout::root(0, LayoutAxis::HORIZONTAL);
        root.leaf(1, size(100, 50));
        // This should panic because we already added a child
        let _root = root.padding(size(10, 10), size(10, 10));
    }

    #[test]
    fn horizontal_container_with_spacing() {
        let mut root = TestLayout::root(0, LayoutAxis::HORIZONTAL).spacing(10);
        root.leaf(1, size(100, 50));
        root.leaf(2, size(80, 60));
        root.leaf(3, size(120, 40));
        let results = root.place(point(0, 0));

        assert_eq!(results.len(), 4);
        // Width: 100 + 10 + 80 + 10 + 120 = 320
        // Height: max(50, 60, 40) = 60
        assert_eq!(results[0], (0, rect(0, 0, 320, 60)));
        // Children in reverse order: 3, 2, 1
        assert_eq!(results[1], (3, rect(200, 0, 120, 40)));
        assert_eq!(results[2], (2, rect(110, 0, 80, 60)));
        assert_eq!(results[3], (1, rect(0, 0, 100, 50)));
    }

    #[test]
    fn spacing_with_padding() {
        let mut root = TestLayout::root(0, LayoutAxis::HORIZONTAL)
            .padding(size(5, 3), size(7, 4))
            .spacing(10);
        root.leaf(1, size(100, 50));
        root.leaf(2, size(80, 60));
        let results = root.place(point(0, 0));

        assert_eq!(results.len(), 3);
        // Width: 5 + 100 + 10 + 80 + 7 = 202
        // Height: 3 + max(50, 60) + 4 = 67
        assert_eq!(results[0], (0, rect(0, 0, 202, 67)));
        // Children in reverse order: 2, 1
        // Second child: 5 (leading) + 100 (first) + 10 (spacing)
        assert_eq!(results[1], (2, rect(115, 3, 80, 60)));
        // First child offset by leading padding
        assert_eq!(results[2], (1, rect(5, 3, 100, 50)));
    }

    #[test]
    #[should_panic(expected = "spacing() must be called before adding any children")]
    fn spacing_after_children_panics() {
        let mut root = TestLayout::root(0, LayoutAxis::HORIZONTAL);
        root.leaf(1, size(100, 50));
        let _root = root.spacing(10);
    }

    impl<const RANK: usize> From<BoxComponents<RANK>> for Box<RANK> {
        fn from(value: BoxComponents<RANK>) -> Self {
            Box::new(value.0.into(), value.1.into())
        }
    }
}
