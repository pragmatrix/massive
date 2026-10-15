use std::fmt::Debug;
use std::hash::Hash;

use derive_more::Constructor;

use crate::dimensional_types::{Offset, Rect, Size};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement<T, const RANK: usize> {
    pub transform: T,
    pub rect: Rect<RANK>,
    pub visible: bool,
}

impl<T, const RANK: usize> Placement<T, RANK> {
    pub fn new(transform: T, rect: Rect<RANK>) -> Self {
        Self {
            transform,
            rect,
            visible: true,
        }
    }

    pub fn with_visibility(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }
}

#[derive(Constructor, Debug, Clone, Copy, PartialEq)]
pub struct TransformOffset<T, const RANK: usize> {
    pub transform: T,
    pub offset: Offset<RANK>,
}

#[derive(Constructor, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasuredLayout<const RANK: usize> {
    pub size: Size<RANK>,
    pub expandable_axes: [bool; RANK],
}

impl<const RANK: usize> From<Size<RANK>> for MeasuredLayout<RANK> {
    fn from(size: Size<RANK>) -> Self {
        Self {
            size,
            expandable_axes: [false; RANK],
        }
    }
}

/// The input and output of placing a parent's children: one placement slot per measured child, so
/// an algorithm can't produce a different count. Algorithms read its fields by name, so the two
/// sizes can't be swapped; `non_exhaustive` keeps other crates from building it past `new`.
#[derive(Debug)]
#[non_exhaustive]
pub struct ChildPlacement<'a, T, const RANK: usize> {
    /// The size the parent is placed at. A re-measured parent places its children once at its
    /// previous size, before its own parent places it again, so for that pass this can be smaller
    /// than `parent_measured_size`.
    pub parent_size: Size<RANK>,
    /// What the parent measured itself as, so placement reuses its measured span instead of
    /// deriving it again; `parent_size` exceeds it by what a stretched parent gained.
    pub parent_measured_size: Size<RANK>,
    pub measurements: &'a [MeasuredLayout<RANK>],
    pub placements: &'a mut [Placement<T, RANK>],
}

impl<'a, T, const RANK: usize> ChildPlacement<'a, T, RANK> {
    pub fn new(
        parent_size: Size<RANK>,
        parent_measured_size: Size<RANK>,
        measurements: &'a [MeasuredLayout<RANK>],
        placements: &'a mut [Placement<T, RANK>],
    ) -> Self {
        assert_eq!(
            measurements.len(),
            placements.len(),
            "one placement slot per child"
        );
        Self {
            parent_size,
            parent_measured_size,
            measurements,
            placements,
        }
    }
}

pub trait LayoutTopology<Id>: Debug
where
    Id: Eq + Hash + Clone,
{
    fn exists(&self, id: &Id) -> bool;
    fn children_of(&self, id: &Id) -> &[Id];
    fn parent_of(&self, id: &Id) -> Option<&Id>;
}

pub trait LayoutAlgorithm<Id, T, const RANK: usize>
where
    Id: Eq + Hash + Clone,
    T: Debug + Copy + PartialEq + Default,
{
    fn measure(&self, id: &Id, child_measurements: &[MeasuredLayout<RANK>])
    -> MeasuredLayout<RANK>;

    /// Writes one placement per child into the slots of `children`.
    fn place_children(&self, id: &Id, children: ChildPlacement<'_, T, RANK>);
}
