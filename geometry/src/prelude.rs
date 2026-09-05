use std::{
    convert,
    ops::{Div, Mul},
};

use euclid::{Size2D, UnknownUnit};

pub trait ComponentDiv<Rhs = Self, Unit = UnknownUnit> {
    type Output;

    fn component_div(self, rhs: Rhs) -> Self::Output;
}

impl<T: Div<Output = T> + Default, U, UR> ComponentDiv<Size2D<T, U>, UR> for Size2D<T, U>
where
    usize: TryFrom<T>,
{
    type Output = Size2D<usize, UR>;

    fn component_div(self, rhs: Self) -> Self::Output {
        Self::Output::from((
            usize::try_from(self.width / rhs.width).unwrap_or_default(),
            usize::try_from(self.height / rhs.height).unwrap_or_default(),
        ))
    }
}

pub trait ComponentMul<Rhs = Self> {
    type Output;

    fn component_mul(self, rhs: Rhs) -> Self::Output;
}

impl<T: Mul<T> + Default, U, U2> ComponentMul<Size2D<usize, U2>> for Size2D<T, U>
where
    euclid::Size2D<T, U>: convert::From<(T::Output, T::Output)>,
    T: convert::TryFrom<usize>,
{
    type Output = Size2D<T, U>;

    fn component_mul(self, rhs: Size2D<usize, U2>) -> Self::Output {
        Self::Output::from((
            self.width * T::try_from(rhs.width).unwrap_or_default(),
            self.height * T::try_from(rhs.height).unwrap_or_default(),
        ))
    }
}

// Robustness: Use num_traits

pub trait SaturatingSub {
    fn saturating_sub(self, rhs: Self) -> Self;
}

impl<U> SaturatingSub for Size2D<u32, U> {
    fn saturating_sub(self, rhs: Self) -> Self {
        Self::new(
            self.width.saturating_sub(rhs.width),
            self.height.saturating_sub(rhs.height),
        )
    }
}
