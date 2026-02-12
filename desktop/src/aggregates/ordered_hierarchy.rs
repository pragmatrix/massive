use std::{collections::HashMap, hash};

use anyhow::{Result, bail};

/// A representation of an ordered hierarchy.
///
/// This implementation does not represent roots. It only maintains relationships between the ids.
#[derive(Debug)]
pub struct OrderedHierarchy<Id> {
    /// Map from Id to parent. Does not contain roots.
    parent: HashMap<Id, Id>,
    /// Map from Id to an ordered list of nested ids. Does not contain roots or empty lists.
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
    /// Add an id of targets to the end of the parent's nested list.
    pub fn add_nested(&mut self, parent: Id, nested: impl IntoIterator<Item = Id>) -> Result<()> {
        for n in nested {
            self.add(parent.clone(), n.clone())?;
        }
        Ok(())
    }

    /// Add an id to the end of the parent's nested list.
    pub fn add(&mut self, parent: Id, nested: Id) -> Result<()> {
        if self.parent.insert(nested.clone(), parent.clone()).is_some() {
            bail!("Internal error: target had already a parent");
        };
        let children = self.nested.entry(parent).or_default();
        children.push(nested);
        Ok(())
    }

    pub fn insert_at(&mut self, parent: Id, index: usize, target: Id) -> Result<()> {
        let children = self.nested.entry(parent.clone()).or_default();

        if index > children.len() {
            anyhow::bail!(
                "Index {} is out of bounds for parent with {} children",
                index,
                children.len()
            );
        }

        if self.parent.insert(target.clone(), parent).is_some() {
            bail!("Internal error: target had already a parent");
        };

        children.insert(index, target);
        Ok(())
    }

    pub fn remove(&mut self, id: &Id) -> Result<()> {
        // Remove this from its parent first.

        if let Some(parent) = self.parent.remove(id)
            && let Some(nested) = self.nested.get_mut(&parent)
        {
            // find + remove should be slightly faster than retain.
            if let Some(index) = nested.iter().position(|nested| nested == id) {
                nested.remove(index);
            } else {
                bail!("Nested not found");
            }
            if nested.is_empty() {
                self.nested.remove(&parent);
            }
        }

        // Remove the complete nested tree. Don't need to care for parents anymore.
        self.remove_nested(id)?;

        Ok(())
    }

    fn remove_nested(&mut self, id: &Id) -> Result<()> {
        let children = self.nested.remove(id).unwrap_or_default();
        for child in children {
            assert!(self.parent.remove(&child).is_some());
            self.remove_nested(&child)?;
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
        let hierarchy = hierarchy();

        assert_eq!(hierarchy.parent(&1), None);
        assert_eq!(hierarchy.nested(&1), &[] as &[i32]);
    }

    #[test]
    fn insert_child_append() {
        let mut hierarchy = hierarchy();
        hierarchy.add(1, 2).unwrap();

        assert_eq!(hierarchy.parent(&2), Some(&1));
        assert_eq!(hierarchy.nested(&1), &[2]);
    }

    #[test]
    fn insert_child_at_index() {
        let mut hierarchy = hierarchy();
        hierarchy.add(1, 2).unwrap();
        hierarchy.insert_at(1, 0, 3).unwrap();

        assert_eq!(hierarchy.nested(&1), &[3, 2]);
    }

    #[test]
    fn insert_invalid_index() {
        let mut hierarchy = hierarchy();

        let result = hierarchy.insert_at(1, 5, 2);
        assert!(result.is_err());
    }

    #[test]
    fn remove_with_nested() {
        let mut hierarchy = hierarchy();
        hierarchy.add(1, 2).unwrap();
        hierarchy.add(2, 3).unwrap();

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
        hierarchy.add(1, 2).unwrap();
        hierarchy.add(1, 3).unwrap();

        hierarchy.remove(&2).unwrap();

        assert_eq!(hierarchy.nested(&1), &[3]);
    }

    #[test]
    fn remove_preserves_order() {
        let mut hierarchy = hierarchy();
        hierarchy.add(1, 2).unwrap();
        hierarchy.add(1, 3).unwrap();
        hierarchy.add(1, 4).unwrap();
        hierarchy.add(1, 5).unwrap();

        hierarchy.remove(&3).unwrap();

        assert_eq!(hierarchy.nested(&1), &[2, 4, 5]);
    }

    fn hierarchy() -> OrderedHierarchy<i32> {
        OrderedHierarchy {
            parent: HashMap::new(),
            nested: HashMap::new(),
        }
    }
}
