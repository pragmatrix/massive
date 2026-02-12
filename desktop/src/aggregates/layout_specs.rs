use std::{any::type_name, collections::HashMap, fmt, hash};

use anyhow::{Result, bail};
use derive_more::Index;

#[derive(Debug, Index)]
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
            bail!("Insertion failed, there is already existing");
        }
        Ok(())
    }

    pub fn insert_or_update(&mut self, key: Key, value: impl Into<Value>) {
        self.map.insert(key, value.into());
    }

    pub fn remove(&mut self, target: &Key) -> Result<()> {
        if self.map.remove(target).is_none() {
            bail!(
                "Can't find target to remove from map of type `{}`",
                type_name::<Value>()
            );
        }
        Ok(())
    }

    pub fn get(&self, target: &Key) -> Option<&Value> {
        self.map.get(target)
    }
}
