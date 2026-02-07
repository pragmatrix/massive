use std::{collections::HashMap, hash};

use anyhow::{Result, bail};

#[derive(Debug)]
pub struct OrderedHierarchy<Id> {
    /// Map from Id to parent.
    parent: HashMap<Id, Id>,
    nested: HashMap<Id, Vec<Id>>,
}

impl<Id> Default for OrderedHierarchy<Id> {
    fn default() -> Self {
        Self {
            parent: Default::default(),
            nested: Default::default(),
        }
    }
}

impl<Id: Clone + Eq + hash::Hash> OrderedHierarchy<Id> {
    /// Add a new root without a parent.
    pub fn add_root(&mut self, target: Id) -> Result<()> {
        self.add(None, target)
    }

    /// Add a target to the end of the parent's nested list.
    pub fn add(&mut self, parent: Option<Id>, target: Id) -> Result<()> {
        if let Some(parent) = parent {
            self.parent.insert(target.clone(), parent.clone());
            let children = self.nested.entry(parent).or_default();
            children.push(target);
        } else {
            assert!(!self.parent.contains_key(&target));
            assert!(!self.nested.contains_key(&target));
        }
        Ok(())
    }

    pub fn insert_at(&mut self, parent: Id, index: usize, target: Id) -> Result<()> {
        if self.parent.insert(target.clone(), parent.clone()).is_some() {
            bail!("Internal error, target had already a parent");
        };
        let children = self.nested.entry(parent).or_default();

        if index > children.len() {
            anyhow::bail!(
                "Index {} is out of bounds for parent with {} children",
                index,
                children.len()
            );
        }

        children.insert(index, target);
        Ok(())
    }

    pub fn remove(&mut self, id: &Id) -> Result<()> {
        self.remove_recursive(id)
    }

    fn remove_recursive(&mut self, id: &Id) -> Result<()> {
        let children = self.nested.remove(id).unwrap_or_default();
        for child in children {
            self.remove_recursive(&child)?;
        }

        if let Some(parent) = self.parent.remove(id)
            && let Some(siblings) = self.nested.get_mut(&parent)
        {
            siblings.retain(|sibling| sibling != id);
        }

        Ok(())
    }

    pub fn parent(&self, target: &Id) -> Option<&Id> {
        self.parent.get(target)
    }

    pub fn nested(&self, target: &Id) -> &[Id] {
        self.nested.get(target).map(Vec::as_slice).unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_root() {
        let mut hierarchy = hierarchy();
        hierarchy.add(None, 1).unwrap();

        assert_eq!(hierarchy.parent(&1), None);
        assert_eq!(hierarchy.nested(&1), &[] as &[i32]);
    }

    #[test]
    fn insert_child_append() {
        let mut hierarchy = hierarchy();
        hierarchy.add(None, 1).unwrap();
        hierarchy.add(Some(1), 2).unwrap();

        assert_eq!(hierarchy.parent(&2), Some(&1));
        assert_eq!(hierarchy.nested(&1), &[2]);
    }

    #[test]
    fn insert_child_at_index() {
        let mut hierarchy = hierarchy();
        hierarchy.add(None, 1).unwrap();
        hierarchy.add(Some(1), 2).unwrap();
        hierarchy.insert_at(1, 0, 3).unwrap();

        assert_eq!(hierarchy.nested(&1), &[3, 2]);
    }

    #[test]
    fn insert_invalid_index() {
        let mut hierarchy = hierarchy();
        hierarchy.add(None, 1).unwrap();

        let result = hierarchy.insert_at(1, 5, 2);
        assert!(result.is_err());
    }

    #[test]
    fn remove_with_nested() {
        let mut hierarchy = hierarchy();
        hierarchy.add(None, 1).unwrap();
        hierarchy.add(Some(1), 2).unwrap();
        hierarchy.add(Some(2), 3).unwrap();

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
        hierarchy.add(None, 1).unwrap();
        hierarchy.add(Some(1), 2).unwrap();
        hierarchy.add(Some(1), 3).unwrap();

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
