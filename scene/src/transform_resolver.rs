use std::collections::HashMap;

use massive_geometry::Transform;

use crate::{Handle, Location};

/// Resolve final transforms from a set of locations.
#[derive(Debug, Default)]
pub struct TransformResolver {
    map: HashMap<Handle<Location>, Transform>,
}

impl TransformResolver {
    pub fn resolve(&mut self, location: &Handle<Location>) -> Transform {
        if let Some(&transform) = self.map.get(location) {
            return transform;
        }

        // Need to extract the parent, so that we don't lock the mutex for too long while going up
        // the hierarchy.
        let (parent, local_transform) = {
            let location_value = location.value();
            (
                location_value.parent.clone(),
                *location_value.transform.value(),
            )
        };

        let resolved = if let Some(parent) = &parent {
            self.resolve(parent) * local_transform
        } else {
            local_transform
        };

        self.map.insert(location.clone(), resolved);
        resolved
    }
}
