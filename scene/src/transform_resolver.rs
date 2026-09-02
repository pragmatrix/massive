use std::collections::HashMap;

use crate::{Location, LocationParent, Ref, ResolvedLocation};

/// Resolve final transforms from a set of locations.
#[derive(Debug, Default)]
pub struct TransformResolver {
    map: HashMap<Ref<Location>, ResolvedLocation>,
}

impl TransformResolver {
    pub fn resolve(&mut self, location: &Ref<Location>) -> ResolvedLocation {
        if let Some(&resolved) = self.map.get(location) {
            return resolved;
        }

        // Need to extract the parent, so that we don't lock the mutex for too long while going up
        // the hierarchy.
        let (parent, local_transform, local_alpha) = {
            let location_value = location.value();
            let parent = match &location_value.parent {
                LocationParent::Location(parent) => parent.clone(),
                LocationParent::Root(space) => {
                    // Roots are terminal: their space is the resolved space.
                    let resolved = ResolvedLocation {
                        transform: *location_value.transform.value(),
                        alpha: location_value.alpha,
                        space: *space,
                    };
                    self.map.insert(location.clone(), resolved);
                    return resolved;
                }
            };
            (
                parent,
                *location_value.transform.value(),
                location_value.alpha,
            )
        };

        // Detail: The only remaining case is a child location; its root space is inherited from
        // the resolved parent, so no local space is needed here.
        let parent_resolved = self.resolve(&parent);
        let resolved = ResolvedLocation {
            transform: parent_resolved.transform * local_transform,
            alpha: parent_resolved.alpha * local_alpha,
            space: parent_resolved.space,
        };

        self.map.insert(location.clone(), resolved);
        resolved
    }
}
