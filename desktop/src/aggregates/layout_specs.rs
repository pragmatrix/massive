use std::{any::type_name, collections::HashMap, hash};

use anyhow::{Result, bail};
use derive_more::Index;

#[derive(Debug, Index)]
pub struct Map<Target, Value> {
    map: HashMap<Target, Value>,
}

impl<Target, Value> Default for Map<Target, Value> {
    fn default() -> Self {
        Self {
            map: Default::default(),
        }
    }
}

impl<Target: Eq + hash::Hash, Value> Map<Target, Value> {
    pub fn insert_or_update(&mut self, target: Target, value: Value) -> Result<()> {
        self.map.insert(target, value);
        Ok(())
    }

    pub fn remove(&mut self, target: &Target) -> Result<()> {
        if self.map.remove(target).is_none() {
            bail!(
                "Can't find target to remove from map of type `{}`",
                type_name::<Value>()
            );
        }
        Ok(())
    }

    pub fn get(&self, target: &Target) -> Option<&Value> {
        self.map.get(target)
    }
}
