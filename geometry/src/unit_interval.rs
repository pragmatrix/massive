use derive_more::{Into, Mul, MulAssign};
use log::warn;

/// 0 .. 1
/// https://english.stackexchange.com/questions/275734/a-word-for-a-value-between-0-and-1-inclusive
#[derive(Copy, Clone, PartialEq, PartialOrd, Debug, Default, Mul, MulAssign, Into)]
pub struct UnitInterval(f64);

impl UnitInterval {
    pub fn new(mut v: f64) -> Self {
        if v.is_nan() {
            warn!("Unit Interval provided with NaN, set to 0.0");
            v = 0.0;
        }
        let clamped = v.clamp(0.0, 1.0);
        if clamped != v {
            warn!("Unit Interval clamped to be in the 0.0..1.0 range, was: {v}");
        }
        Self(clamped)
    }

    pub const fn new_unchecked(v: f64) -> Self {
        Self(v)
    }
}
