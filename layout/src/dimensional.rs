use massive_geometry::{PointPx, RectPx, SizePx};

pub trait DimensionalOffset: Copy {
    const RANK: usize;
    fn get(&self, i: usize) -> i32;
    fn set(&mut self, i: usize, value: i32);
    fn zero() -> Self;
}

pub trait DimensionalSize: Copy {
    const RANK: usize;
    fn get(&self, i: usize) -> u32;
    fn set(&mut self, i: usize, value: u32);
    fn empty() -> Self;
}

pub trait DimensionalRect: Copy {
    type Size: DimensionalSize;
    type Offset: DimensionalOffset;

    const RANK: usize;
    fn get(&self, i: usize) -> i32;
    fn set(&mut self, i: usize, value: i32);
    fn empty() -> Self;
    fn from_offset_size(offset: Self::Offset, size: Self::Size) -> Self;
}

impl DimensionalSize for SizePx {
    const RANK: usize = 2;

    fn get(&self, i: usize) -> u32 {
        match i {
            0 => self.width,
            1 => self.height,
            _ => panic!("Index out of bounds for 2D size"),
        }
    }

    fn set(&mut self, i: usize, value: u32) {
        match i {
            0 => self.width = value,
            1 => self.height = value,
            _ => panic!("Index out of bounds for 2D size"),
        }
    }

    fn empty() -> Self {
        Self::new(0, 0)
    }
}

impl DimensionalOffset for PointPx {
    const RANK: usize = 2;

    fn get(&self, i: usize) -> i32 {
        match i {
            0 => self.x,
            1 => self.y,
            _ => panic!("Index out of bounds for 2D offset"),
        }
    }

    fn set(&mut self, i: usize, value: i32) {
        match i {
            0 => self.x = value,
            1 => self.y = value,
            _ => panic!("Index out of bounds for 2D offset"),
        }
    }

    fn zero() -> Self {
        Self::new(0, 0)
    }
}

impl DimensionalRect for RectPx {
    type Size = SizePx;
    type Offset = PointPx;

    const RANK: usize = 2;

    fn get(&self, i: usize) -> i32 {
        self.origin.get(i)
    }

    fn set(&mut self, i: usize, value: i32) {
        self.origin.set(i, value);
    }

    fn empty() -> Self {
        Self::zero()
    }

    fn from_offset_size(offset: Self::Offset, size: Self::Size) -> Self {
        Self::new(offset, size.cast())
    }
}
