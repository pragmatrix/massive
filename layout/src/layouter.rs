//! Second attempt at the layout engine.
//!
//! This is a non-trait based layouter that supports providing an id, and spits out a number of
//! absolute `(Id,Rect)` pairs.

use std::{cmp::max, mem};

use crate::{
    LayoutAxis,
    dimensional2::{Box, Offset, Size},
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
    offset: Offset<RANK>,
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
            parent.child(self.id.clone(), self.inner.size, self.inner.children);
        }
    }
}

pub type BoxExt<const RANK: usize> = ([i32; RANK], [u32; RANK]);

impl<const RANK: usize> From<Box<RANK>> for BoxExt<RANK> {
    fn from(value: Box<RANK>) -> Self {
        (value.offset.0, value.size.0)
    }
}

impl<const RANK: usize> From<BoxExt<RANK>> for Box<RANK> {
    fn from(value: BoxExt<RANK>) -> Self {
        Box::new(value.0.into(), value.1.into())
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

    pub fn size(&self) -> Size<RANK> {
        self.inner.size
    }

    pub fn place<BX>(self, absolute_offset: impl Into<[i32; RANK]>) -> Vec<(Id, BX)>
    where
        BX: From<BoxExt<RANK>>,
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
        BX: From<BoxExt<RANK>>,
    {
        if self.parent.is_some() {
            panic!("Layout finalization can only be done on root containers");
        }

        let mut out = |(id, bx): (Id, Box<RANK>)| {
            let ext_box: BoxExt<RANK> = bx.into();
            out((id, ext_box.into()))
        };

        let entry = TraceEntry {
            id: self.id.clone(),
            bx: Box::new(Offset::ZERO, self.size()),
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
        let child_relative_box = Box::new(self.offset, child_size);
        self.trace.push(TraceEntry {
            id,
            bx: child_relative_box,
            children,
        });

        let axis = *self.layout_axis;
        let child_size_at_axis = child_size[axis] as i32;
        let current_offset = self.offset[axis];
        self.offset[axis] = current_offset + child_size_at_axis;

        for i in 0..RANK {
            if i == axis {
                let current = self.size[i];
                let child_val = child_size[i];
                self.size[i] = current + child_val;
            } else {
                let current = self.size[i];
                let child_val = child_size[i];
                self.size[i] = max(current, child_val);
            }
        }
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
}
