use derive_more::{Deref, From, Into};

/// A path into a focus tree / hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Deref, From, Into)]
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
    pub fn transition(&mut self, other: Self) -> Vec<FocusPathTransition<T>>
    where
        T: PartialEq + Clone,
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
            transitions.push(FocusPathTransition::Exit(self[..=i].to_vec().into()));
        }

        // Generate enter transitions top-down (from common ancestor to leaf)
        // Enter indices from common_prefix_len to (new_path.len() - 1)
        for i in common_prefix_len..other.len() {
            transitions.push(FocusPathTransition::Enter(other[..=i].to_vec().into()));
        }

        *self = other;

        transitions
    }
}

#[derive(Debug)]
pub enum FocusPathTransition<T> {
    Exit(FocusPath<T>),
    Enter(FocusPath<T>),
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

    fn format_transitions<T: fmt::Debug>(transitions: &[FocusPathTransition<T>]) -> String {
        transitions
            .iter()
            .map(|t| match t {
                FocusPathTransition::Enter(path) => format!("Enter{:?}", &**path),
                FocusPathTransition::Exit(path) => format!("Exit{:?}", &**path),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn test_focus<T: PartialEq + Clone + fmt::Debug>(from: Vec<T>, to: Vec<T>, expected: &str) {
        let mut path: FocusPath<T> = from.into();
        let transitions = path.transition(to.into());
        assert_eq!(format_transitions(&transitions), expected);
    }
}
