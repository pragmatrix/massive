use std::{collections::HashMap, hash};

use anyhow::Result;

use crate::event_sourcing::OrderedInsertion;

#[derive(Debug)]
pub struct OrderedHierarchy<Target> {
    parent: HashMap<Target, Target>,
    nested: HashMap<Target, Vec<Target>>,
}

impl<Target> Default for OrderedHierarchy<Target> {
    fn default() -> Self {
        Self {
            parent: Default::default(),
            nested: Default::default(),
        }
    }
}

#[derive(Debug)]
pub enum OrderedHierarchyCommand<T> {
    Insert(T, Option<OrderedInsertion<T>>),
    /// Removes T and all nested ones.
    Remove(T),
}

impl<T> OrderedHierarchyCommand<T> {
    pub fn map<NT>(self, f: impl Fn(T) -> NT) -> OrderedHierarchyCommand<NT> {
        match self {
            Self::Insert(parent, ordered_insertion) => {
                OrderedHierarchyCommand::Insert(f(parent), ordered_insertion.map(|i| i.map(f)))
            }
            Self::Remove(id) => OrderedHierarchyCommand::Remove(f(id)),
        }
    }
}

impl<T: Clone + Eq + hash::Hash> OrderedHierarchy<T> {
    pub fn apply(&mut self, command: OrderedHierarchyCommand<T>) -> Result<()> {
        match command {
            OrderedHierarchyCommand::Insert(target, ordered_insertion) => {
                self.insert(target, ordered_insertion)
            }
            OrderedHierarchyCommand::Remove(target) => self.remove(&target),
        }
    }

    pub fn insert_mapped<F>(
        &mut self,
        target: F,
        ordered_insertion: Option<OrderedInsertion<F>>,
        f: impl Fn(F) -> T,
    ) -> Result<()> {
        self.insert(f(target), ordered_insertion.map(|oi| oi.map(f)))
    }

    pub fn insert(
        &mut self,
        target: T,
        ordered_insertion: impl Into<Option<OrderedInsertion<T>>>,
    ) -> Result<()> {
        if let Some(ordered_insertion) = ordered_insertion.into() {
            let parent = ordered_insertion.parent().clone();
            self.parent.insert(target.clone(), parent.clone());
            let children = self.nested.entry(parent).or_default();
            match ordered_insertion {
                OrderedInsertion::Insert(_, index) => {
                    if index > children.len() {
                        anyhow::bail!(
                            "invalid insertion index: {} (children count: {})",
                            index,
                            children.len()
                        );
                    }
                    children.insert(index, target);
                }
                OrderedInsertion::Append(_) => {
                    children.push(target);
                }
            }
        }
        Ok(())
    }

    pub fn remove(&mut self, target: &T) -> Result<()> {
        self.remove_recursive(target)
    }

    fn remove_recursive(&mut self, target: &T) -> Result<()> {
        let children = self.nested.remove(target).unwrap_or_default();
        for child in children {
            self.remove_recursive(&child)?;
        }

        if let Some(parent) = self.parent.remove(target)
            && let Some(siblings) = self.nested.get_mut(&parent)
        {
            siblings.retain(|sibling| sibling != target);
        }

        Ok(())
    }

    pub fn parent(&self, target: &T) -> Option<&T> {
        self.parent.get(target)
    }

    pub fn nested(&self, target: &T) -> &[T] {
        self.nested.get(target).map(Vec::as_slice).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_root() {
        let mut hierarchy = hierarchy();
        hierarchy.insert(1, None).unwrap();

        assert_eq!(hierarchy.parent(&1), None);
        assert_eq!(hierarchy.nested(&1), &[] as &[i32]);
    }

    #[test]
    fn insert_child_append() {
        let mut hierarchy = hierarchy();
        hierarchy.insert(1, None).unwrap();
        hierarchy
            .insert(2, Some(OrderedInsertion::Append(1)))
            .unwrap();

        assert_eq!(hierarchy.parent(&2), Some(&1));
        assert_eq!(hierarchy.nested(&1), &[2]);
    }

    #[test]
    fn insert_child_at_index() {
        let mut hierarchy = hierarchy();
        hierarchy.insert(1, None).unwrap();
        hierarchy
            .insert(2, Some(OrderedInsertion::Append(1)))
            .unwrap();
        hierarchy
            .insert(3, Some(OrderedInsertion::Insert(1, 0)))
            .unwrap();

        assert_eq!(hierarchy.nested(&1), &[3, 2]);
    }

    #[test]
    fn insert_invalid_index() {
        let mut hierarchy = hierarchy();
        hierarchy.insert(1, None).unwrap();

        let result = hierarchy.insert(2, Some(OrderedInsertion::Insert(1, 5)));
        assert!(result.is_err());
    }

    #[test]
    fn remove_with_nested() {
        let mut hierarchy = hierarchy();
        hierarchy.insert(1, None).unwrap();
        hierarchy
            .insert(2, Some(OrderedInsertion::Append(1)))
            .unwrap();
        hierarchy
            .insert(3, Some(OrderedInsertion::Append(2)))
            .unwrap();

        hierarchy.remove(&1).unwrap();

        assert_eq!(hierarchy.parent(&1), None);
        assert_eq!(hierarchy.parent(&2), None);
        assert_eq!(hierarchy.parent(&3), None);
        assert_eq!(hierarchy.nested(&1), &[] as &[i32]);
        assert_eq!(hierarchy.nested(&2), &[] as &[i32]);
    }

    #[test]
    fn remove_removes_from_parent_list() {
        let mut hierarchy = hierarchy();
        hierarchy.insert(1, None).unwrap();
        hierarchy
            .insert(2, Some(OrderedInsertion::Append(1)))
            .unwrap();
        hierarchy
            .insert(3, Some(OrderedInsertion::Append(1)))
            .unwrap();

        hierarchy.remove(&2).unwrap();

        assert_eq!(hierarchy.nested(&1), &[3]);
    }

    fn hierarchy() -> OrderedHierarchy<i32> {
        OrderedHierarchy {
            parent: HashMap::new(),
            nested: HashMap::new(),
        }
    }
}
