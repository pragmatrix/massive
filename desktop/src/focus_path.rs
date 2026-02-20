use std::{fmt, hash, iter};

use derive_more::{Deref, From, Into};

use crate::OrderedHierarchy;

/// A path into a focus tree / hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Deref, From, Into)]
pub struct FocusPath<T>(Vec<T>);

impl<T: PartialEq> Default for FocusPath<T> {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl<T: PartialEq> FocusPath<T> {
    pub const EMPTY: Self = Self(Vec::new());

    pub fn new(component: impl Into<T>) -> Self {
        Self([component.into()].into())
    }

    // Idea: Add a trait that confirms that a joining path is structurally allowed.
    pub fn join(mut self, component: impl Into<T>) -> Self {
        self.0.push(component.into());
        self
    }

    pub fn parent(&self) -> Option<Self>
    where
        T: Clone,
    {
        self.0.split_last().map(|(_, rest)| rest.to_vec().into())
    }

    /// Transition to a new path and return the nested exit / enter sequence.
    #[must_use]
    pub fn transitions(&self, other: Self) -> Vec<FocusTransition<T>>
    where
        T: Clone,
    {
        // Find common prefix length where both paths match
        let common_prefix_len = (*self)
            .iter()
            .zip(other.iter())
            .take_while(|(a, b)| a == b)
            .count();

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

        transitions
    }
}

#[derive(Debug)]
pub enum FocusTransition<T> {
    Exit(T),
    Enter(T),
}

pub trait PathResolver<Id: Clone> {
    fn parent(&self, id: &Id) -> Option<&Id>;

    fn resolve_path<'a>(&'a self, id: impl Into<Option<&'a Id>>) -> FocusPath<Id>
    where
        Id: 'a,
    {
        let mut v: Vec<_> = iter::successors(id.into(), |id| self.parent(id))
            .cloned()
            .collect();
        v.reverse();
        v.into()
    }
}

impl<Id: fmt::Debug + Clone + Eq + hash::Hash> PathResolver<Id> for OrderedHierarchy<Id> {
    fn parent(&self, id: &Id) -> Option<&Id> {
        self.parent(id)
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;

    use super::*;

    #[test]
    fn test_focus_transitions() {
        test_focus(vec![1, 2, 3], vec![1, 2, 3], "");
        test_focus(vec![1, 2], vec![1, 3], "Exit2, Enter3");
        test_focus(vec![1], vec![1, 2, 3], "Enter2, Enter3");
        test_focus(vec![1, 2, 3], vec![1], "Exit3, Exit2");
        test_focus(vec![1, 2], vec![3, 4], "Exit2, Exit1, Enter3, Enter4");
        test_focus(
            vec!['A', 'B', 'C', 'D'],
            vec!['A', 'B', 'E', 'F'],
            "Exit'D', Exit'C', Enter'E', Enter'F'",
        );
        test_focus(vec![], vec![1, 2], "Enter1, Enter2");
        test_focus(vec![1, 2], vec![], "Exit2, Exit1");
    }

    fn format_transitions<T: fmt::Debug>(transitions: &[FocusTransition<T>]) -> String {
        transitions
            .iter()
            .map(|t| match t {
                FocusTransition::Enter(target) => format!("Enter{:?}", target),
                FocusTransition::Exit(target) => format!("Exit{:?}", target),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn test_focus<T: PartialEq + Clone + fmt::Debug>(from: Vec<T>, to: Vec<T>, expected: &str) {
        let path: FocusPath<T> = from.into();
        let transitions = path.transitions(to.into());
        assert_eq!(format_transitions(&transitions), expected);
    }
}
