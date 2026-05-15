use std::fmt::Debug;
use std::hash::Hash;

use derive_more::Constructor;

use crate::dimensional_types::{Offset, Rect, Size};

#[derive(Constructor, Debug, Clone, Copy, PartialEq)]
pub struct Placement<T, const RANK: usize> {
    pub transform: T,
    pub rect: Rect<RANK>,
}

#[derive(Constructor, Debug, Clone, Copy, PartialEq)]
pub struct TransformOffset<T, const RANK: usize> {
    pub transform: T,
    pub offset: Offset<RANK>,
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
    fn measure(&self, id: &Id, child_sizes: &[Size<RANK>]) -> Size<RANK>;

    fn place_children(&self, id: &Id, child_sizes: &[Size<RANK>]) -> Vec<TransformOffset<T, RANK>>;
}
