use derive_more::{Deref, From, Into};

mod dimensional_types;
mod layouter;

pub use dimensional_types::{Box, Offset, Size, Thickness};
pub use layouter::{ContainerBuilder, Layout, container, leaf};

#[derive(Debug, Copy, Clone, From, Into, Deref, Default)]
pub struct LayoutAxis(usize);

impl LayoutAxis {
    pub const HORIZONTAL: Self = Self(0);
    pub const VERTICAL: Self = Self(1);
    pub const DEPTH: Self = Self(2);
}

mod geometry_interop {
    use massive_geometry::{PointPx, RectPx, SizePx};

    use crate::{Box, Offset, Size};

    impl From<SizePx> for Size<2> {
        fn from(value: SizePx) -> Self {
            [value.width, value.height].into()
        }
    }

    impl From<PointPx> for Offset<2> {
        fn from(value: PointPx) -> Self {
            [value.x, value.y].into()
        }
    }

    impl From<Box<2>> for RectPx {
        fn from(value: Box<2>) -> Self {
            let [x, y] = value.offset.into();
            let [w, h] = value.size.into();
            RectPx::new((x, y).into(), (w as i32, h as i32).into())
        }
    }
}
