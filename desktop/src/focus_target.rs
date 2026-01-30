#![allow(unused)]
use std::{fmt::Debug, iter};

use derive_more::Deref;

pub trait FocusTarget: Debug + PartialEq + Clone {
    fn parent(&self) -> Option<Self>;
}

#[derive(Debug, Deref)]
pub struct FocusPath<T>(Vec<T>);

#[derive(Debug)]
pub enum FocusTransition<T> {
    Exit(T),
    Enter(T),
}

pub trait ToHierarchy: Sized {
    fn to_hierarchy(&self) -> FocusPath<Self>;
    fn transition(&mut self, other: Self) -> Vec<FocusTransition<Self>>;
}

impl<T: FocusTarget> ToHierarchy for T {
    fn to_hierarchy(&self) -> FocusPath<Self> {
        FocusPath::from(self.clone())
    }

    fn transition(&mut self, other: Self) -> Vec<FocusTransition<T>> {
        // Find common prefix length where both paths match
        let mut from_h = self.to_hierarchy();
        let to_h = other.to_hierarchy();
        let transitions = from_h.transition(to_h);
        *self = other;
        transitions
    }
}

impl<T: FocusTarget> From<T> for FocusPath<T> {
    fn from(target: T) -> Self {
        let mut path: Vec<_> = iter::successors(Some(target), |t| t.parent()).collect();
        path.reverse();
        Self(path)
    }
}

impl<T: FocusTarget> FocusPath<T> {
    pub fn target(&self) -> &T {
        self.last().expect("Internal error: No focus target")
    }

    /// Transition to a new path and return the nested exit / enter sequence.
    #[must_use]
    pub fn transition(&mut self, other: Self) -> Vec<FocusTransition<T>>
    where
        T: PartialEq + Clone,
    {
        // Find common prefix length where both paths match
        let common_prefix_len = self.common_prefix_len(&other);

        let mut transitions = Vec::new();

        // Generate exit transitions bottom-up (from leaf to common ancestor)
        // Exit indices from (self.focused.len() - 1) down to common_prefix_len
        for i in (common_prefix_len..self.len()).rev() {
            transitions.push(FocusTransition::Exit(self[i].clone()));
        }

        // Generate enter transitions top-down (from common ancestor to leaf)
        // Enter indices from common_prefix_len to (new_path.len() - 1)
        for i in common_prefix_len..other.len() {
            transitions.push(FocusTransition::Enter(other[i].clone()));
        }

        *self = other;

        transitions
    }

    fn common_prefix_len(&self, other: &Self) -> usize {
        // Find common prefix length where both paths match
        (*self)
            .iter()
            .zip(other.iter())
            .take_while(|(a, b)| a == b)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestTarget(Vec<i32>);

    impl FocusTarget for TestTarget {
        fn parent(&self) -> Option<Self> {
            self.0.split_last().map(|(_, rest)| Self(rest.to_vec()))
        }
    }

    #[test]
    fn test_focus_transitions() {
        test_focus(vec![1, 2, 3], vec![1, 2, 3], "");
        test_focus(vec![1, 2], vec![1, 3], "Exit[1, 2], Enter[1, 3]");
        test_focus(vec![1], vec![1, 2, 3], "Enter[1, 2], Enter[1, 2, 3]");
        test_focus(vec![1, 2, 3], vec![1], "Exit[1, 2, 3], Exit[1, 2]");
        test_focus(
            vec![1, 2],
            vec![3, 4],
            "Exit[1, 2], Exit[1], Enter[3], Enter[3, 4]",
        );
        test_focus(vec![], vec![1, 2], "Enter[1], Enter[1, 2]");
        test_focus(vec![1, 2], vec![], "Exit[1, 2], Exit[1]");
    }

    fn format_transitions(transitions: &[FocusTransition<TestTarget>]) -> String {
        transitions
            .iter()
            .map(|t| match t {
                FocusTransition::Enter(target) => format!("Enter{:?}", target.0),
                FocusTransition::Exit(target) => format!("Exit{:?}", target.0),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn test_focus(from: Vec<i32>, to: Vec<i32>, expected: &str) {
        let mut target = TestTarget(from);
        let transitions = target.transition(TestTarget(to));
        assert_eq!(format_transitions(&transitions), expected);
    }
}
