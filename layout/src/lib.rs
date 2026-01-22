use derive_more::{Deref, From, Into};

mod dimensional;
mod dimensional_types;
mod layouter;

pub use layouter::{BoxComponents as Box, Layouter};

#[derive(Debug, Copy, Clone, From, Into, Deref, Default)]
pub struct LayoutAxis(usize);

impl LayoutAxis {
    pub const HORIZONTAL: Self = Self(0);
    pub const VERTICAL: Self = Self(1);
    pub const DEPTH: Self = Self(2);
}
