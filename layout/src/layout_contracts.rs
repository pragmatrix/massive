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

pub trait LayoutTopology<Id>
where
    Id: Eq + Hash + Clone,
{
    fn exists(&self, id: &Id) -> bool;
    fn children_of(&self, id: &Id) -> &[Id];
    fn parent_of(&self, id: &Id) -> Option<Id>;
}

pub trait LayoutAlgorithm<Id, T, const RANK: usize>
where
    Id: Eq + Hash + Clone,
    T: Debug + Copy + PartialEq + Default,
{
    fn measure(&self, id: &Id, child_measurements: &[MeasuredLayout<RANK>])
    -> MeasuredLayout<RANK>;

    fn place_children(
        &self,
        id: &Id,
        parent_size: Size<RANK>,
        child_measurements: &[MeasuredLayout<RANK>],
    ) -> Vec<Placement<T, RANK>>;
}
