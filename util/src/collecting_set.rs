use std::collections::{HashSet, hash_set};
use std::hash::Hash;
use std::mem;
use std::ops::{Add, AddAssign};

/// A set that collects values, deduplicating as they arrive.
///
/// Storage stays compact: no allocation for zero or one value, a `HashSet`
/// only once a second distinct value appears.
///
/// When constructed via this type's APIs (`insert`, `Add`, `AddAssign`, etc.), storage reflects
/// the distinct element count, which keeps equality consistent with set semantics.
#[must_use]
#[derive(Debug, Clone)]
pub struct CollectingSet<T>(CollectingSetStorage<T>);

#[derive(Debug, Clone)]
enum CollectingSetStorage<T> {
    Empty,
    One(T),
    Many(HashSet<T>),
}

// Manual equality: `HashSet<T>: PartialEq`/`Eq` needs `T: Eq + Hash`, which a
// derive would not require on the type parameter.
impl<T: Eq + Hash> PartialEq for CollectingSet<T> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (CollectingSetStorage::Empty, CollectingSetStorage::Empty) => true,
            (CollectingSetStorage::One(a), CollectingSetStorage::One(b)) => a == b,
            (CollectingSetStorage::Many(a), CollectingSetStorage::Many(b)) => a == b,
            _ => false,
        }
    }
}

impl<T: Eq + Hash> Eq for CollectingSet<T> {}

impl<T> CollectingSet<T> {
    pub fn is_empty(&self) -> bool {
        matches!(self.0, CollectingSetStorage::Empty)
    }

    pub fn len(&self) -> usize {
        match &self.0 {
            CollectingSetStorage::Empty => 0,
            CollectingSetStorage::One(_) => 1,
            CollectingSetStorage::Many(values) => values.len(),
        }
    }
}

impl<T: Eq + Hash> CollectingSet<T> {
    pub fn insert(&mut self, value: T) {
        match &mut self.0 {
            CollectingSetStorage::Empty => self.0 = CollectingSetStorage::One(value),
            CollectingSetStorage::One(existing) => {
                if *existing != value {
                    let CollectingSetStorage::One(existing) =
                        mem::replace(&mut self.0, CollectingSetStorage::Empty)
                    else {
                        unreachable!()
                    };
                    let mut set = HashSet::with_capacity(2);
                    set.insert(existing);
                    set.insert(value);
                    self.0 = CollectingSetStorage::Many(set);
                }
            }
            CollectingSetStorage::Many(set) => {
                set.insert(value);
            }
        }
    }

    pub fn retain(&mut self, mut predicate: impl FnMut(&T) -> bool) {
        match &mut self.0 {
            CollectingSetStorage::Empty => {}
            CollectingSetStorage::One(value) => {
                if !predicate(value) {
                    self.0 = CollectingSetStorage::Empty;
                }
            }
            CollectingSetStorage::Many(values) => {
                values.retain(predicate);
                match values.len() {
                    0 => self.0 = CollectingSetStorage::Empty,
                    1 => {
                        let value = values
                            .drain()
                            .next()
                            .expect("a set with one value must yield that value");
                        self.0 = CollectingSetStorage::One(value);
                    }
                    _ => {}
                }
            }
        }
    }
}

impl<T: Eq + Hash> AddAssign<T> for CollectingSet<T> {
    fn add_assign(&mut self, value: T) {
        self.insert(value);
    }
}

impl<T: Eq + Hash> AddAssign for CollectingSet<T> {
    fn add_assign(&mut self, other: Self) {
        for value in other {
            self.insert(value);
        }
    }
}

impl<T: Eq + Hash> Add<T> for CollectingSet<T> {
    type Output = Self;

    fn add(mut self, value: T) -> Self {
        self += value;
        self
    }
}

impl<T: Eq + Hash> Add for CollectingSet<T> {
    type Output = Self;

    fn add(mut self, other: Self) -> Self {
        self += other;
        self
    }
}

// Manual to avoid the `T: Default` bound a derive would add.
#[allow(clippy::derivable_impls)]
impl<T> Default for CollectingSet<T> {
    fn default() -> Self {
        Self(CollectingSetStorage::Empty)
    }
}

impl<T> From<T> for CollectingSet<T> {
    fn from(value: T) -> Self {
        Self(CollectingSetStorage::One(value))
    }
}

impl<T: Eq + Hash> FromIterator<T> for CollectingSet<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut set = CollectingSet::default();
        for value in iter {
            set.insert(value);
        }
        set
    }
}

impl<T> IntoIterator for CollectingSet<T> {
    type Item = T;
    type IntoIter = CollectingSetIntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        match self.0 {
            CollectingSetStorage::Empty => CollectingSetIntoIter::Empty,
            CollectingSetStorage::One(value) => CollectingSetIntoIter::One(Some(value)),
            CollectingSetStorage::Many(set) => CollectingSetIntoIter::Many(set.into_iter()),
        }
    }
}

pub enum CollectingSetIntoIter<T> {
    Empty,
    One(Option<T>),
    Many(hash_set::IntoIter<T>),
}

impl<T> Iterator for CollectingSetIntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        match self {
            CollectingSetIntoIter::Empty => None,
            CollectingSetIntoIter::One(value) => value.take(),
            CollectingSetIntoIter::Many(iter) => iter.next(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_tracks_distinct_element_count() {
        let mut set = CollectingSet::default();
        assert!(set.is_empty());

        set.insert(1);
        assert_eq!(set.len(), 1);
        assert_eq!(set, CollectingSet::from(1));

        set.insert(2);
        assert_eq!(sorted(set), vec![1, 2]);
    }

    #[test]
    fn insert_deduplicates() {
        let mut set = CollectingSet::default();
        set.insert(1);
        set.insert(1);
        assert_eq!(set, CollectingSet::from(1));

        set.insert(2);
        set.insert(2);
        set.insert(1);
        assert_eq!(sorted(set), vec![1, 2]);
    }

    #[test]
    fn add_assign_value_and_set() {
        let mut set = CollectingSet::default();
        set += 1;
        set += 2;
        set += CollectingSet::from(2);
        set += many([3, 4]);
        assert_eq!(sorted(set), vec![1, 2, 3, 4]);
    }

    #[test]
    fn add_combines_without_mutating_operands() {
        let combined = CollectingSet::from(1) + 2 + many([2, 3]);
        assert_eq!(sorted(combined), vec![1, 2, 3]);
    }

    #[test]
    fn into_iter_yields_all_values_once() {
        assert!(CollectingSet::<i32>::default().into_iter().next().is_none());
        assert_eq!(sorted(CollectingSet::from(7)), vec![7]);
        assert_eq!(sorted(many([1, 2, 3])), vec![1, 2, 3]);
    }

    fn many<const N: usize>(values: [i32; N]) -> CollectingSet<i32> {
        let mut set = CollectingSet::default();
        for value in values {
            set.insert(value);
        }
        set
    }

    fn sorted(set: CollectingSet<i32>) -> Vec<i32> {
        let mut values: Vec<i32> = set.into_iter().collect();
        values.sort_unstable();
        values
    }
}
