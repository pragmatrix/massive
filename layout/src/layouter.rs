//! Second attempt at the layout engine.
//!
//! This is a non-trait based one, that supports providing an id, and spits out a number of absolute
//! `(Id,Rect)` pairs.

use std::{cmp::max, mem};

use crate::{
    LayoutAxis,
    dimensional::{DimensionalOffset, DimensionalRect, DimensionalSize},
};

#[derive(Debug)]
struct Layout<'a, Id: Copy, R: DimensionalRect> {
    id: Id,
    parent: Option<&'a mut LayoutInner<Id, R>>,
    inner: LayoutInner<Id, R>,
}

#[derive(Debug)]
struct LayoutInner<Id: Copy, R: DimensionalRect> {
    trace: Vec<TraceEntry<Id, R>>,
    layout_axis: LayoutAxis,
    offset: R::Offset,
    size: R::Size,
    children: usize,
}

#[derive(Debug)]
struct TraceEntry<Id, R> {
    id: Id,
    rect: R,
    /// The number of children, 0 if this is a leaf.
    children: usize,
}

impl<Id: Copy, R: DimensionalRect> Drop for Layout<'_, Id, R> {
    fn drop(&mut self) {
        if let Some(parent) = self.parent.take() {
            parent.trace = mem::take(&mut self.inner.trace);
            parent.child(self.id, self.inner.size, self.inner.children);
        }
    }
}

impl<'a, Id: Copy, R: DimensionalRect> Layout<'a, Id, R> {
    pub fn root(id: Id, layout_axis: LayoutAxis) -> Self {
        Self::new(None, id, layout_axis)
    }

    fn new(
        mut parent: Option<&'a mut LayoutInner<Id, R>>,
        id: Id,
        layout_axis: LayoutAxis,
    ) -> Self {
        // If there is a parent, get the trace.
        let trace = parent
            .as_mut()
            .map_or_else(Vec::new, |parent| mem::take(&mut parent.trace));

        Self {
            id,
            parent,
            inner: LayoutInner {
                trace,
                layout_axis,
                offset: R::Offset::zero(),
                size: R::Size::empty(),
                children: 0,
            },
        }
    }

    fn leaf(&mut self, id: Id, child_size: R::Size) {
        self.inner.child(id, child_size, 0);
    }

    pub fn container<'b>(&'b mut self, id: Id, layout_axis: LayoutAxis) -> Layout<'b, Id, R> {
        Layout::new(Some(&mut self.inner), id, layout_axis)
    }

    pub fn size(&self) -> R::Size {
        self.inner.size
    }

    pub fn place(mut self, absolute_offset: R::Offset) -> Vec<(Id, R)> {
        if self.parent.is_some() {
            panic!("Layout finalization can only be done on root containers");
        }

        let mut out = Vec::new();
        let entry = TraceEntry {
            id: self.id,
            rect: R::from_offset_size(R::Offset::zero(), self.size()),
            children: self.inner.children,
        };
        place_rec(&mut self.inner.trace, absolute_offset, entry, &mut out);
        out
    }
}

impl<Id: Copy, R: DimensionalRect> LayoutInner<Id, R> {
    fn child(&mut self, id: Id, child_size: R::Size, children: usize) {
        let child_relative_rect = R::from_offset_size(self.offset, child_size);
        self.trace.push(TraceEntry {
            id,
            rect: child_relative_rect,
            children,
        });

        let axis = *self.layout_axis;
        let child_size_at_axis = child_size.get(axis) as i32;
        let current_offset = self.offset.get(axis);
        self.offset.set(axis, current_offset + child_size_at_axis);

        let rank = R::Size::RANK;
        for i in 0..rank {
            if i == axis {
                let current = self.size.get(i);
                let child_val = child_size.get(i);
                self.size.set(i, current + child_val);
            } else {
                let current = self.size.get(i);
                let child_val = child_size.get(i);
                self.size.set(i, max(current, child_val));
            }
        }
        self.children += 1;
    }
}

fn place_rec<Id, R: DimensionalRect>(
    trace: &mut Vec<TraceEntry<Id, R>>,
    offset: R::Offset,
    this: TraceEntry<Id, R>,
    out: &mut Vec<(Id, R)>,
) {
    let absolute_rect = add_offset(this.rect, offset);
    out.push((this.id, absolute_rect));
    let children_offset = absolute_rect.offset();

    for _ in 0..this.children {
        let child = trace
            .pop()
            .expect("Internal error: Trace of children does not match");
        place_rec(trace, children_offset, child, out);
    }
}

fn add_offset<R: DimensionalRect>(mut rect: R, offset: R::Offset) -> R {
    let rank = R::RANK;
    for i in 0..rank {
        let pos = rect.get(i);
        let offset = offset.get(i);
        rect.set(i, pos + offset);
    }
    rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use massive_geometry::{PointPx, RectPx, SizePx};

    type TestLayout<'a> = Layout<'a, u32, RectPx>;

    #[test]
    fn single_leaf() {
        let mut root = TestLayout::root(0, LayoutAxis::HORIZONTAL);
        root.leaf(1, size(100, 50));
        let results = root.place(PointPx::new(0, 0));

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
        let results = root.place(PointPx::new(0, 0));

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
        let results = root.place(PointPx::new(0, 0));

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
        let results = root.place(PointPx::new(0, 0));

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], (0, rect(0, 0, 0, 0)));
    }

    #[test]
    fn custom_offset() {
        let mut root = TestLayout::root(0, LayoutAxis::HORIZONTAL);
        root.leaf(1, size(10, 10));
        root.leaf(2, size(20, 20));
        let results = root.place(PointPx::new(100, 200));

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
        let results = root.place(PointPx::new(0, 0));

        // Along horizontal axis: widths sum (10+20+30=60)
        // Perpendicular (vertical): heights max (50)
        assert_eq!(results[0], (0, rect(0, 0, 60, 50)));
    }

    #[test]
    #[should_panic(expected = "Index out of bounds")]
    fn depth_axis_on_2d_rect_panics() {
        // Test that using DEPTH axis (index 2) on 2D rects properly panics
        // since RectPx only has rank 2 (width and height)
        let mut root = TestLayout::root(0, LayoutAxis::DEPTH);
        root.leaf(1, size(100, 200)); // This should panic
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

        let results = root.place(PointPx::new(0, 0));

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

    // Helper to create RectPx from unsigned size
    fn rect(x: i32, y: i32, w: u32, h: u32) -> RectPx {
        RectPx::new(PointPx::new(x, y), SizePx::new(w, h).cast())
    }

    // Helper to create SizePx
    fn size(w: u32, h: u32) -> SizePx {
        SizePx::new(w, h)
    }
}
