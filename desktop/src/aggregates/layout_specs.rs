use std::{any::type_name, collections::HashMap, fmt, hash};

use anyhow::{Result, bail};
use derive_more::{Deref, Index};

#[derive(Debug, Index, Deref)]
pub struct Map<Key, Value> {
    map: HashMap<Key, Value>,
}

impl<Key, Value> Default for Map<Key, Value> {
    fn default() -> Self {
        Self {
            map: Default::default(),
        }
    }
}

impl<Key: fmt::Debug + Eq + hash::Hash, Value: Sized> Map<Key, Value> {
    pub fn insert(&mut self, key: Key, value: impl Into<Value>) -> Result<()> {
        if self.map.insert(key, value.into()).is_some() {
            bail!("Insertion failed: key already exists in map");
        }
        Ok(())
    }

    pub fn insert_or_update(&mut self, key: Key, value: impl Into<Value>) {
        self.map.insert(key, value.into());
    }

    pub fn remove(&mut self, key: &Key) -> Result<()> {
        if self.map.remove(key).is_none() {
            bail!(
                "Can't find key to remove from map of type `{}`",
                type_name::<Value>()
            );
        }
        Ok(())
    }

    pub fn get_mut(&mut self, key: &Key) -> Option<&mut Value> {
        self.map.get_mut(key)
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Value> {
        self.map.values_mut()
    }
}
