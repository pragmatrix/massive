//! The FocusTree manages a current focus path that points into a focus tree / hierarchy and
//! provides the necessary event transitions when the focus changes.
use std::mem;

use derive_more::{Deref, From, Into};

#[derive(Debug)]
pub struct FocusTree<T> {
    focused: FocusPath<T>,
}

impl<T> Default for FocusTree<T> {
    fn default() -> Self {
        Self {
            focused: Default::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deref, From, Into)]
pub struct FocusPath<T>(Vec<T>);

impl<T> Default for FocusPath<T> {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl<T> FocusPath<T> {
    pub const EMPTY: Self = Self(Vec::new());

    pub fn new(component: impl Into<T>) -> Self {
        Self([component.into()].into())
    }

    pub fn push(&mut self, component: impl Into<T>) {
        self.0.push(component.into())
    }

    pub fn with(mut self, component: impl Into<T>) -> Self {
        self.push(component.into());
        self
    }

    pub fn take(&mut self) -> Self {
        mem::take(self)
    }
}

pub type FocusTransitions<T> = Vec<FocusTransition<T>>;

#[derive(Debug)]
pub enum FocusTransition<T> {
    Enter(FocusPath<T>),
    Exit(FocusPath<T>),
}

impl<T> FocusTree<T> {
    pub fn focused(&self) -> &FocusPath<T> {
        &self.focused
    }

    pub fn focus(&mut self, new_path: FocusPath<T>) -> FocusTransitions<T>
    where
        T: PartialEq + Clone,
    {
        let mut transitions = Vec::new();

        // Find common prefix length where both paths match
        let common_prefix_len = self
            .focused
            .iter()
            .zip(new_path.iter())
            .take_while(|(a, b)| a == b)
            .count();

        // Generate exit transitions bottom-up (from leaf to common ancestor)
        // Exit indices from (self.focused.len() - 1) down to common_prefix_len
        for i in (common_prefix_len..self.focused.len()).rev() {
            transitions.push(FocusTransition::Exit(self.focused[..=i].to_vec().into()));
        }

        // Generate enter transitions top-down (from common ancestor to leaf)
        // Enter indices from common_prefix_len to (new_path.len() - 1)
        for i in common_prefix_len..new_path.len() {
            transitions.push(FocusTransition::Enter(new_path[..=i].to_vec().into()));
        }

        // Update current focus
        self.focused = new_path;

        transitions
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;

    use super::*;

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
        test_focus(
            vec!['A', 'B', 'C', 'D'],
            vec!['A', 'B', 'E', 'F'],
            "Exit['A', 'B', 'C', 'D'], Exit['A', 'B', 'C'], Enter['A', 'B', 'E'], Enter['A', 'B', 'E', 'F']",
        );
        test_focus(vec![], vec![1, 2], "Enter[1], Enter[1, 2]");
        test_focus(vec![1, 2], vec![], "Exit[1, 2], Exit[1]");
    }

    fn format_transitions<T: fmt::Debug>(transitions: &[FocusTransition<T>]) -> String {
        transitions
            .iter()
            .map(|t| match t {
                FocusTransition::Enter(path) => format!("Enter{:?}", &**path),
                FocusTransition::Exit(path) => format!("Exit{:?}", &**path),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn test_focus<T: PartialEq + Clone + fmt::Debug>(from: Vec<T>, to: Vec<T>, expected: &str) {
        let mut tree = FocusTree {
            focused: from.into(),
        };
        let transitions = tree.focus(to.into());
        assert_eq!(format_transitions(&transitions), expected);
    }
}
