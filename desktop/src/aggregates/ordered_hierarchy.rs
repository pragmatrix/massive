use std::{collections::HashMap, fmt, hash};

use anyhow::{Result, bail};

/// A representation of an ordered hierarchy.
///
/// The hierarchy keeps one map with per-node state so existence checks can be answered directly
/// from a single source of truth.
#[derive(Debug)]
pub struct OrderedHierarchy<Id> {
    /// Map from Id to node state.
    nodes: HashMap<Id, NodeState<Id>>,
}

#[derive(Debug)]
struct NodeState<Id> {
    parent: Option<Id>,
    nested: Vec<Id>,
}

impl<Id> Default for NodeState<Id> {
    fn default() -> Self {
        Self {
            parent: None,
            nested: Vec::new(),
        }
    }
}

impl<Id> Default for OrderedHierarchy<Id> {
    fn default() -> Self {
        Self {
            nodes: Default::default(),
        }
    }
}

impl<Id> OrderedHierarchy<Id>
where
    Id: fmt::Debug + Clone + Eq + hash::Hash,
{
    /// Add an id of targets to the end of the parent's nested list.
    pub fn add_nested(&mut self, parent: Id, nested: impl IntoIterator<Item = Id>) -> Result<()> {
        for n in nested {
            self.add(parent.clone(), n.clone())?;
        }
        Ok(())
    }

    /// Add an id to the end of the parent's nested list.
    pub fn add(&mut self, parent: Id, nested: Id) -> Result<()> {
        let nested_state = self.nodes.entry(nested.clone()).or_default();
        if nested_state.parent.is_some() {
            bail!("Internal error (add): nested {nested:?} had already a parent");
        }
        nested_state.parent = Some(parent.clone());

        self.nodes.entry(parent).or_default().nested.push(nested);
        Ok(())
    }

    pub fn insert_at(&mut self, parent: Id, index: usize, nested: Id) -> Result<()> {
        let nested_list = &self.nodes.entry(parent.clone()).or_default().nested;

        if index > nested_list.len() {
            anyhow::bail!(
                "Index {index} is out of bounds for parent with {} nested items",
                nested_list.len()
            );
        }

        let nested_state = self.nodes.entry(nested.clone()).or_default();
        if nested_state.parent.is_some() {
            bail!("Internal error (insert_at): nested {nested:?} had already a parent");
        }
        nested_state.parent = Some(parent.clone());

        self.nodes
            .entry(parent)
            .or_default()
            .nested
            .insert(index, nested);
        Ok(())
    }

    pub fn remove(&mut self, id: &Id) -> Result<()> {
        let parent = if let Some(node) = self.nodes.get(id) {
            node.parent.clone()
        } else {
            bail!("Internal error (remove): id {id:?} not found");
        };

        // Remove this from its parent first.

        if let Some(parent) = parent.as_ref() {
            let nested = &mut self
                .nodes
                .get_mut(parent)
                .unwrap_or_else(|| {
                    panic!("Internal error (remove): parent {parent:?} of id {id:?} not found")
                })
                .nested;

            // find + remove should be slightly faster than retain.
            if let Some(index) = nested.iter().position(|nested| nested == id) {
                nested.remove(index);
            } else {
                bail!("Nested not found");
            }
        }

        // Remove the complete nested tree.
        self.remove_nested_with_expected_parent(id, parent.as_ref())?;

        Ok(())
    }

    fn remove_nested_with_expected_parent(
        &mut self,
        id: &Id,
        expected_parent: Option<&Id>,
    ) -> Result<()> {
        let node = self
            .nodes
            .remove(id)
            .unwrap_or_else(|| panic!("Internal error (remove_nested): id {id:?} not found"));

        debug_assert_eq!(
            node.parent.as_ref(),
            expected_parent,
            "Internal error (remove_nested): nested item {id:?} had parent {:?}, expected {:?}",
            node.parent,
            expected_parent
        );

        for nested_item in node.nested {
            self.remove_nested_with_expected_parent(&nested_item, Some(id))?;
        }

        Ok(())
    }

    pub fn parent(&self, id: &Id) -> Option<&Id> {
        self.nodes.get(id).and_then(|node| node.parent.as_ref())
    }

    /// Check whether a node exists in the hierarchy.
    pub fn exists(&self, id: &Id) -> bool {
        self.nodes.contains_key(id)
    }

    pub fn get_nested(&self, id: &Id) -> &[Id] {
        self.nodes
            .get(id)
            .map(|node| node.nested.as_slice())
            .unwrap_or(&[])
    }

    /// Return all items that share the same parent as `id`.
    ///
    /// The returned slice includes `id` itself when `id` is non-root.
    pub fn group(&self, id: &Id) -> &[Id] {
        self.parent(id)
            .map(|parent| self.get_nested(parent))
            .unwrap_or(&[])
    }

    pub fn entry<'a>(&'a self, id: &'a Id) -> Entry<'a, Id> {
        Entry {
            hierarchy: self,
            id,
        }
    }
}

#[derive(derive_more::Debug)]
pub struct Entry<'a, Id> {
    #[debug(skip)]
    hierarchy: &'a OrderedHierarchy<Id>,
    id: &'a Id,
}

impl<Id> Entry<'_, Id>
where
    Id: fmt::Debug + Clone + Eq + hash::Hash,
{
    pub fn parent(&self) -> Option<&Id> {
        self.hierarchy.parent(self.id)
    }

    pub fn has_nested(&self) -> bool {
        !self.hierarchy.get_nested(self.id).is_empty()
    }

    pub fn nested(&self) -> &[Id] {
        self.hierarchy.get_nested(self.id)
    }

    pub fn index(&self) -> Option<usize> {
        self.hierarchy
            .get_nested(self.parent()?)
            .iter()
            .position(|nested| nested == self.id)
    }

    pub fn group(&self) -> &[Id] {
        self.hierarchy.group(self.id)
    }

    /// Find the next neighbor in the given preferred direction, if there is none in the direction
    /// takes one in the opposite side. None if there is none.
    pub fn neighbor(&self, direction_bias: DirectionBias) -> Option<&Id> {
        let index = self.index()?;
        let group = self.group();
        match index {
            0 => group.get(1),
            i if i == group.len() - 1 => group.get(i - 1),
            i => match direction_bias {
                DirectionBias::Begin => group.get(i - 1),
                DirectionBias::End => group.get(i + 1),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionBias {
    Begin,
    End,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_root() {
        let hierarchy = hierarchy();

        assert_eq!(hierarchy.parent(&1), None);
        assert_eq!(hierarchy.get_nested(&1), &[] as &[i32]);
    }

    #[test]
    fn insert_nested_append() {
        let mut hierarchy = hierarchy();
        hierarchy.add(1, 2).unwrap();

        assert_eq!(hierarchy.parent(&2), Some(&1));
        assert_eq!(hierarchy.get_nested(&1), &[2]);
    }

    #[test]
    fn insert_nested_at_index() {
        let mut hierarchy = hierarchy();
        hierarchy.add(1, 2).unwrap();
        hierarchy.insert_at(1, 0, 3).unwrap();

        assert_eq!(hierarchy.get_nested(&1), &[3, 2]);
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
        assert_eq!(hierarchy.get_nested(&1), &[] as &[i32]);
        assert_eq!(hierarchy.get_nested(&2), &[] as &[i32]);
    }

    #[test]
    fn remove_removes_from_parent_list() {
        let mut hierarchy = hierarchy();
        hierarchy.add(1, 2).unwrap();
        hierarchy.add(1, 3).unwrap();

        hierarchy.remove(&2).unwrap();

        assert_eq!(hierarchy.get_nested(&1), &[3]);
    }

    #[test]
    fn remove_preserves_order() {
        let mut hierarchy = hierarchy();
        hierarchy.add(1, 2).unwrap();
        hierarchy.add(1, 3).unwrap();
        hierarchy.add(1, 4).unwrap();
        hierarchy.add(1, 5).unwrap();

        hierarchy.remove(&3).unwrap();

        assert_eq!(hierarchy.get_nested(&1), &[2, 4, 5]);
    }

    #[test]
    fn exists_for_empty_parent_after_nested_remove() {
        let mut hierarchy = hierarchy();
        hierarchy.add(1, 2).unwrap();

        hierarchy.remove(&2).unwrap();

        assert!(hierarchy.exists(&1));
    }

    #[test]
    fn group_returns_siblings_with_self() {
        let mut hierarchy = hierarchy();
        hierarchy.add(1, 2).unwrap();
        hierarchy.add(1, 3).unwrap();
        hierarchy.add(1, 4).unwrap();

        assert_eq!(hierarchy.group(&3), &[2, 3, 4]);
    }

    #[test]
    fn group_for_root_is_empty() {
        let hierarchy = hierarchy();

        assert_eq!(hierarchy.group(&1), &[] as &[i32]);
    }

    fn hierarchy() -> OrderedHierarchy<i32> {
        OrderedHierarchy {
            nodes: HashMap::new(),
        }
    }
}
