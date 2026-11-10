use std::collections::HashMap;

use massive_scene::{
    Change, Id, Location, LocationParent, ResolvedLocation, SceneChange, Transform,
};

/// Resolve final transforms of locations from a batch of submitted scene changes, for tests that
/// check where the scene draws a target.
///
/// It resolves what the renderer would receive, by walking the parent ids of the submitted
/// locations. Client code holds only handles and references, which cannot walk to a parent's
/// value (ADR 0040). The resolved alpha is sanitized, invariant for every [`ResolvedLocation`],
/// including the cached ones.
#[derive(Debug)]
pub struct TransformResolver {
    transforms: HashMap<Id<Transform>, Transform>,
    locations: HashMap<Id<Location>, Location>,
    resolved: HashMap<Id<Location>, ResolvedLocation>,
}

impl TransformResolver {
    /// Collect the latest state of each transform and location that `changes` creates or updates.
    pub fn new(changes: &[SceneChange]) -> Self {
        let mut transforms = HashMap::new();
        let mut locations = HashMap::new();
        for change in changes {
            match change {
                SceneChange::Transform(
                    Change::Create(id, transform) | Change::Update(id, transform),
                ) => {
                    transforms.insert(*id, *transform);
                }
                SceneChange::Location(
                    Change::Create(id, location) | Change::Update(id, location),
                ) => {
                    locations.insert(*id, *location);
                }
                SceneChange::Transform(Change::Retain(_) | Change::Release(_))
                | SceneChange::Location(Change::Retain(_) | Change::Release(_))
                | SceneChange::Visual(_) => {}
            }
        }

        Self {
            transforms,
            locations,
            resolved: HashMap::new(),
        }
    }

    /// Resolve the location with the id `location`, which the changes must have submitted, as must
    /// every location above it.
    pub fn resolve(&mut self, location: Id<Location>) -> ResolvedLocation {
        if let Some(&resolved) = self.resolved.get(&location) {
            return resolved;
        }

        let location_value = *self
            .locations
            .get(&location)
            .unwrap_or_else(|| panic!("Location {location:?} was not submitted"));
        let local_transform = *self
            .transforms
            .get(&location_value.transform)
            .unwrap_or_else(|| {
                panic!("Transform {:?} was not submitted", location_value.transform)
            });
        let local_alpha = location_value.sanitized_alpha();

        let resolved = match location_value.parent {
            // Roots are terminal: their space is the resolved space.
            LocationParent::Root(space) => ResolvedLocation {
                transform: local_transform,
                alpha: local_alpha,
                space,
            },
            // A child inherits its root space from the resolved parent.
            LocationParent::Location(parent) => {
                let parent_resolved = self.resolve(parent);
                ResolvedLocation {
                    transform: parent_resolved.transform * local_transform,
                    alpha: parent_resolved.alpha * local_alpha,
                    space: parent_resolved.space,
                }
            }
        };

        self.resolved.insert(location, resolved);
        resolved
    }
}
