//! Second iteration of dimension. Just use ranked types.
//!
//! Another idea is just to use typedefs, but then it wouldn't be possible to implement functions on
//! top of them we might need.

use std::ops;

use derive_more::{Add, From, Index, IndexMut};

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct Box<const RANK: usize> {
    pub offset: Offset<RANK>,
    pub size: Size<RANK>,
}

impl<const RANK: usize> Box<RANK> {
    pub const fn new(offset: Offset<RANK>, size: Size<RANK>) -> Self {
        Self { offset, size }
    }

    pub const EMPTY: Self = Self {
        offset: Offset::ZERO,
        size: Size::EMPTY,
    };
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct Thickness<const RANK: usize> {
    pub leading: Size<RANK>,
    pub trailing: Size<RANK>,
}

impl<const RANK: usize> Thickness<RANK> {
    pub const ZERO: Self = Self {
        leading: Size::EMPTY,
        trailing: Size::EMPTY,
    };
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Index, IndexMut, From)]
pub struct Offset<const RANK: usize>(pub [i32; RANK]);

impl<const RANK: usize> Default for Offset<RANK> {
    fn default() -> Self {
        Self::ZERO
    }
}

impl<const RANK: usize> Offset<RANK> {
    pub const ZERO: Self = Self([0; RANK]);
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Index, IndexMut, From)]
pub struct Size<const RANK: usize>(pub [u32; RANK]);

impl<const RANK: usize> Default for Size<RANK> {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl<const RANK: usize> Size<RANK> {
    pub const EMPTY: Self = Self([0; RANK]);
}

impl<const RANK: usize> ops::AddAssign for Size<RANK> {
    fn add_assign(&mut self, rhs: Self) {
        for i in 0..RANK {
            self[i] += rhs[i]
        }
    }
}

impl<const RANK: usize> ops::Add for Size<RANK> {
    type Output = Self;

    fn add(mut self, rhs: Self) -> Self::Output {
        self += rhs;
        self
    }
}
