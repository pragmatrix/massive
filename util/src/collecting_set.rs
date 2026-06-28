use std::collections::{HashSet, hash_set};
use std::hash::Hash;
use std::mem;
use std::ops::{Add, AddAssign};

/// A set that collects values, deduplicating as they arrive.
///
/// Storage stays compact: no allocation for zero or one value, a `HashSet`
/// only once a second distinct value appears. The variant always reflects the
/// element count (`Empty` = 0, `One` = 1, `Many` >= 2), which keeps the derived
/// equality consistent with set semantics.
#[must_use]
#[derive(Debug, Clone)]
pub enum CollectingSet<T> {
    Empty,
    One(T),
    Many(HashSet<T>),
}

// Manual equality: `HashSet<T>: PartialEq`/`Eq` needs `T: Eq + Hash`, which a
// derive would not require on the type parameter.
impl<T: Eq + Hash> PartialEq for CollectingSet<T> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CollectingSet::Empty, CollectingSet::Empty) => true,
            (CollectingSet::One(a), CollectingSet::One(b)) => a == b,
            (CollectingSet::Many(a), CollectingSet::Many(b)) => a == b,
            _ => false,
        }
    }
}

impl<T: Eq + Hash> Eq for CollectingSet<T> {}

impl<T: Eq + Hash> CollectingSet<T> {
    pub fn insert(&mut self, value: T) {
        match self {
            CollectingSet::Empty => *self = CollectingSet::One(value),
            CollectingSet::One(existing) => {
                if *existing != value {
                    let CollectingSet::One(existing) = mem::replace(self, CollectingSet::Empty)
                    else {
                        unreachable!()
                    };
                    let mut set = HashSet::with_capacity(2);
                    set.insert(existing);
                    set.insert(value);
                    *self = CollectingSet::Many(set);
                }
            }
            CollectingSet::Many(set) => {
                set.insert(value);
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
        CollectingSet::Empty
    }
}

impl<T> From<T> for CollectingSet<T> {
    fn from(value: T) -> Self {
        CollectingSet::One(value)
    }
}

impl<T> IntoIterator for CollectingSet<T> {
    type Item = T;
    type IntoIter = CollectingSetIntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            CollectingSet::Empty => CollectingSetIntoIter::Empty,
            CollectingSet::One(value) => CollectingSetIntoIter::One(Some(value)),
            CollectingSet::Many(set) => CollectingSetIntoIter::Many(set.into_iter()),
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
    fn insert_transitions_through_variants() {
        let mut set = CollectingSet::default();
        assert_eq!(set, CollectingSet::Empty);

        set.insert(1);
        assert_eq!(set, CollectingSet::One(1));

        set.insert(2);
        assert_eq!(sorted(set), vec![1, 2]);
    }

    #[test]
    fn insert_deduplicates() {
        let mut set = CollectingSet::Empty;
        set.insert(1);
        set.insert(1);
        assert_eq!(set, CollectingSet::One(1));

        set.insert(2);
        set.insert(2);
        set.insert(1);
        assert_eq!(sorted(set), vec![1, 2]);
    }

    #[test]
    fn add_assign_value_and_set() {
        let mut set = CollectingSet::Empty;
        set += 1;
        set += 2;
        set += CollectingSet::One(2);
        set += many([3, 4]);
        assert_eq!(sorted(set), vec![1, 2, 3, 4]);
    }

    #[test]
    fn add_combines_without_mutating_operands() {
        let combined = CollectingSet::One(1) + 2 + many([2, 3]);
        assert_eq!(sorted(combined), vec![1, 2, 3]);
    }

    #[test]
    fn into_iter_yields_all_values_once() {
        assert!(CollectingSet::<i32>::Empty.into_iter().next().is_none());
        assert_eq!(sorted(CollectingSet::One(7)), vec![7]);
        assert_eq!(sorted(many([1, 2, 3])), vec![1, 2, 3]);
    }

    fn many<const N: usize>(values: [i32; N]) -> CollectingSet<i32> {
        let mut set = CollectingSet::Empty;
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
