use std::mem;
use std::ops::{Add, AddAssign};
use std::vec;

/// A sequence that collects values in insertion order, keeping duplicates.
///
/// Storage stays compact: no allocation for zero or one value, a `Vec` only
/// once a second value appears. The variant always reflects the element count
/// (`Empty` = 0, `One` = 1, `Many` >= 2).
#[must_use]
#[derive(Debug, Clone)]
pub enum CollectingVec<T> {
    Empty,
    One(T),
    Many(Vec<T>),
}

impl<T> CollectingVec<T> {
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    pub fn push(&mut self, value: impl Into<T>) {
        let value = value.into();
        match self {
            CollectingVec::Empty => *self = CollectingVec::One(value),
            CollectingVec::One(_) => {
                let CollectingVec::One(existing) = mem::replace(self, CollectingVec::Empty) else {
                    unreachable!()
                };
                *self = CollectingVec::Many(vec![existing, value]);
            }
            CollectingVec::Many(values) => values.push(value),
        }
    }

    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> CollectingVec<U> {
        match self {
            CollectingVec::Empty => CollectingVec::Empty,
            CollectingVec::One(value) => CollectingVec::One(f(value)),
            CollectingVec::Many(values) => CollectingVec::Many(values.into_iter().map(f).collect()),
        }
    }
}

impl<T> AddAssign<T> for CollectingVec<T> {
    fn add_assign(&mut self, value: T) {
        self.push(value);
    }
}

impl<T> AddAssign for CollectingVec<T> {
    fn add_assign(&mut self, other: Self) {
        self.extend(other);
    }
}

impl<T, const LEN: usize> AddAssign<[T; LEN]> for CollectingVec<T> {
    fn add_assign(&mut self, values: [T; LEN]) {
        self.extend(values);
    }
}

impl<T> Extend<T> for CollectingVec<T> {
    // Reserve from the iterator's size hint so a bulk merge grows the `Vec` at
    // most once instead of reallocating per element.
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        let incoming = iter.into_iter();
        match self {
            CollectingVec::Many(values) => {
                values.reserve(incoming.size_hint().0);
                values.extend(incoming);
            }
            // `None`/`One` hold 0 or 1 values; combine them with the incoming
            // stream and pick the variant from the first two combined elements.
            _ => {
                let existing = mem::replace(self, CollectingVec::Empty);
                let mut combined = existing.into_iter().chain(incoming);
                let Some(first) = combined.next() else { return };
                let Some(second) = combined.next() else {
                    *self = CollectingVec::One(first);
                    return;
                };
                let mut values = Vec::with_capacity(combined.size_hint().0 + 2);
                values.push(first);
                values.push(second);
                values.extend(combined);
                *self = CollectingVec::Many(values);
            }
        }
    }
}

impl<T> Add<T> for CollectingVec<T> {
    type Output = Self;

    fn add(mut self, value: T) -> Self {
        self += value;
        self
    }
}

impl<T> Add for CollectingVec<T> {
    type Output = Self;

    fn add(mut self, other: Self) -> Self {
        self += other;
        self
    }
}

impl<T, const LEN: usize> Add<[T; LEN]> for CollectingVec<T> {
    type Output = Self;

    fn add(mut self, values: [T; LEN]) -> Self {
        self += values;
        self
    }
}

// Manual to avoid the `T: Default` bound a derive would add.
#[allow(clippy::derivable_impls)]
impl<T> Default for CollectingVec<T> {
    fn default() -> Self {
        CollectingVec::Empty
    }
}

impl<T> From<T> for CollectingVec<T> {
    fn from(value: T) -> Self {
        CollectingVec::One(value)
    }
}

impl<T, const LEN: usize> From<[T; LEN]> for CollectingVec<T> {
    fn from(values: [T; LEN]) -> Self {
        match LEN {
            0 => CollectingVec::Empty,
            1 => CollectingVec::One(values.into_iter().next().unwrap()),
            _ => CollectingVec::Many(values.into()),
        }
    }
}

impl<T> From<Vec<T>> for CollectingVec<T> {
    fn from(values: Vec<T>) -> Self {
        match values.len() {
            0 => CollectingVec::Empty,
            1 => CollectingVec::One(values.into_iter().next().unwrap()),
            _ => CollectingVec::Many(values),
        }
    }
}

impl<T> IntoIterator for CollectingVec<T> {
    type Item = T;
    type IntoIter = CollectingVecIntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            CollectingVec::Empty => CollectingVecIntoIter::Empty,
            CollectingVec::One(value) => CollectingVecIntoIter::One(Some(value)),
            CollectingVec::Many(values) => CollectingVecIntoIter::Many(values.into_iter()),
        }
    }
}

pub enum CollectingVecIntoIter<T> {
    Empty,
    One(Option<T>),
    Many(vec::IntoIter<T>),
}

impl<T> Iterator for CollectingVecIntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        match self {
            CollectingVecIntoIter::Empty => None,
            CollectingVecIntoIter::One(value) => value.take(),
            CollectingVecIntoIter::Many(iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            CollectingVecIntoIter::Empty => (0, Some(0)),
            CollectingVecIntoIter::One(value) => {
                let len = value.is_some() as usize;
                (len, Some(len))
            }
            CollectingVecIntoIter::Many(iter) => iter.size_hint(),
        }
    }
}

impl<T> ExactSizeIterator for CollectingVecIntoIter<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_transitions_through_variants() {
        let mut values = CollectingVec::default();
        assert!(matches!(values, CollectingVec::Empty));

        values.push(1);
        assert!(matches!(values, CollectingVec::One(1)));

        values.push(2);
        assert_eq!(collected(values), vec![1, 2]);
    }

    #[test]
    fn insert_keeps_duplicates_in_order() {
        let mut values = CollectingVec::Empty;
        values.push(2);
        values.push(1);
        values.push(2);
        values.push(1);
        assert_eq!(collected(values), vec![2, 1, 2, 1]);
    }

    #[test]
    fn add_assign_value_and_sequence() {
        let mut values = CollectingVec::Empty;
        values += 1;
        values += 2;
        values += CollectingVec::One(2);
        values += many([3, 4]);
        assert_eq!(collected(values), vec![1, 2, 2, 3, 4]);
    }

    #[test]
    fn add_combines_without_mutating_operands() {
        let combined = CollectingVec::One(1) + 2 + many([2, 3]);
        assert_eq!(collected(combined), vec![1, 2, 2, 3]);
    }

    #[test]
    fn into_iter_yields_all_values_once() {
        assert!(CollectingVec::<i32>::Empty.into_iter().next().is_none());
        assert_eq!(collected(CollectingVec::One(7)), vec![7]);
        assert_eq!(collected(many([1, 2, 3])), vec![1, 2, 3]);
    }

    fn many<const N: usize>(values: [i32; N]) -> CollectingVec<i32> {
        values.into()
    }

    fn collected(values: CollectingVec<i32>) -> Vec<i32> {
        values.into_iter().collect()
    }
}
