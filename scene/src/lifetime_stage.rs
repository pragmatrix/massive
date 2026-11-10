use std::any::TypeId;

use massive_geometry::Transform;

use crate::{
    Change, Id, Location, Object, RenderChange, RenderSceneChange, SceneChange, Visual,
    id_generator,
};

/// Decides when scene objects end (ADR 0040).
///
/// The stage sits between the queues that collect scene changes and the renderer. It keeps a
/// reference count for each object. The object's handle counts as one reference, every counted
/// [`Ref`](crate::Ref) as one more, and so does every live object that refers to it by id. When the
/// count reaches zero, the stage emits the object's delete, returns its id to the id generator,
/// and releases what the object referred to, which may end those objects in turn.
///
/// Each batch stays whole: an object released in a batch is deleted in that same batch. The renderer
/// relies on that, because it resolves references only between batches.
#[derive(Debug, Default)]
pub struct LifetimeStage {
    transforms: Table,
    locations: Table,
    visuals: Table,
}

impl LifetimeStage {
    /// Resolve one drained batch into the changes the renderer applies.
    pub fn resolve(&mut self, changes: Vec<SceneChange>) -> Vec<RenderSceneChange> {
        // Most changes resolve into one render change, so the batch's length is a good first
        // capacity.
        let mut resolution = Resolution {
            render_changes: Vec::with_capacity(changes.len()),
            ended: Vec::new(),
        };

        for change in changes {
            match change {
                SceneChange::Transform(change) => self.resolve_change(change, &mut resolution),
                SceneChange::Location(change) => self.resolve_change(change, &mut resolution),
                SceneChange::Visual(change) => self.resolve_change(change, &mut resolution),
            }
        }

        id_generator::release_all(&resolution.ended);
        resolution.render_changes
    }

    fn resolve_change<T: Tracked>(&mut self, change: Change<T>, resolution: &mut Resolution)
    where
        SceneChange: From<Change<T>>,
        RenderSceneChange: From<RenderChange<T>>,
    {
        match change {
            Change::Create(id, value) => {
                let targets = value.targets();
                self.retain_all(targets);
                let table = self.table(T::KIND);
                table.counts.create(id.index());
                table.targets.set(id.index(), targets);
                resolution.emit(RenderChange::Create(id, value));
            }
            Change::Update(id, value) => {
                let targets = value.targets();
                let table = self.table(T::KIND);
                debug_assert!(
                    table.counts.is_alive(id.index()),
                    "Updated object {id:?} is not alive"
                );
                let previous = table.targets.replace(id.index(), targets);
                if targets != previous {
                    // The new targets are counted before the previous ones are released, so a
                    // target both refer to, such as a transform kept under a new parent, never
                    // reaches zero in between.
                    self.retain_all(targets);
                    self.release_all(previous.iter(), resolution);
                }
                resolution.emit(RenderChange::Update(id, value));
            }
            Change::Retain(id) => self.table(T::KIND).counts.retain(id.index()),
            Change::Release(id) => self.release_all([Target::from(id)], resolution),
        }
    }

    fn retain_all(&mut self, targets: Targets) {
        for target in targets.iter() {
            self.table(target.kind).counts.retain(target.index);
        }
    }

    /// Release references, and end every object whose count reaches zero, together with what
    /// it referred to.
    ///
    /// A worklist instead of recursion, because a chain of locations can be arbitrarily long.
    fn release_all(
        &mut self,
        targets: impl IntoIterator<Item = Target>,
        resolution: &mut Resolution,
    ) {
        let mut pending: Vec<Target> = targets.into_iter().collect();
        while let Some(Target { kind, index }) = pending.pop() {
            let table = self.table(kind);
            if table.counts.release(index) {
                pending.extend(table.targets.take(index).iter());
                resolution.end(kind, index);
            }
        }
    }

    fn table(&mut self, kind: Kind) -> &mut Table {
        match kind {
            Kind::Transform => &mut self.transforms,
            Kind::Location => &mut self.locations,
            Kind::Visual => &mut self.visuals,
        }
    }
}

/// The lifetime stage's view of an object: its kind, and what it refers to.
pub trait Tracked: Object
where
    SceneChange: From<Change<Self>>,
{
    const KIND: Kind;

    /// The objects this value refers to. The stage counts them for as long as the object lives.
    fn targets(&self) -> Targets;
}

/// The kind of an object, which selects its table in the stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Transform,
    Location,
    Visual,
}

impl Kind {
    /// The object type the id generator hands this kind's ids out under.
    fn type_id(self) -> TypeId {
        match self {
            Kind::Transform => TypeId::of::<Transform>(),
            Kind::Location => TypeId::of::<Location>(),
            Kind::Visual => TypeId::of::<Visual>(),
        }
    }

    fn delete(self, index: u32) -> RenderSceneChange {
        match self {
            Kind::Transform => RenderChange::Delete(Id::<Transform>::from_index(index)).into(),
            Kind::Location => RenderChange::Delete(Id::<Location>::from_index(index)).into(),
            Kind::Visual => RenderChange::Delete(Id::<Visual>::from_index(index)).into(),
        }
    }
}

/// An object that one reference count belongs to.
///
/// It holds the kind and the raw index instead of a typed id, because one worklist holds targets
/// of every kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    kind: Kind,
    index: u32,
}

impl<T: Tracked> From<Id<T>> for Target
where
    SceneChange: From<Change<T>>,
{
    fn from(object: Id<T>) -> Self {
        Self {
            kind: T::KIND,
            index: object.index(),
        }
    }
}

/// What an object refers to. Two at most: a location refers to its transform and, unless it is a
/// root, to its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Targets([Option<Target>; 2]);

impl Targets {
    pub fn new(targets: [Option<Target>; 2]) -> Self {
        Self(targets)
    }

    fn iter(self) -> impl Iterator<Item = Target> {
        self.0.into_iter().flatten()
    }
}

/// The counts and the targets of every object of one kind.
#[derive(Debug, Default)]
struct Table {
    counts: ReferenceCounts,
    /// What each live object refers to, from its latest create or update.
    targets: Slots<Targets>,
}

/// What resolving one batch produces: the render changes, and the ids of the objects that ended.
#[derive(Debug)]
struct Resolution {
    render_changes: Vec<RenderSceneChange>,
    /// Keyed by the object type, which is what the id generator hands ids out under.
    ended: Vec<(TypeId, u32)>,
}

impl Resolution {
    fn emit<T>(&mut self, change: RenderChange<T>)
    where
        RenderSceneChange: From<RenderChange<T>>,
    {
        self.render_changes.push(change.into());
    }

    fn end(&mut self, kind: Kind, index: u32) {
        self.ended.push((kind.type_id(), index));
        self.render_changes.push(kind.delete(index));
    }
}

/// The reference count of each object of one kind, indexed by the index of the object's id. Zero marks an id
/// without a live object.
#[derive(Debug, Default)]
struct ReferenceCounts(Vec<u32>);

impl ReferenceCounts {
    /// Start a new object's count at one, for its handle.
    fn create(&mut self, index: u32) {
        let index = index as usize;
        if index >= self.0.len() {
            self.0.resize(index + 1, 0);
        }
        assert_eq!(self.0[index], 0, "Created object {index} is already alive");
        self.0[index] = 1;
    }

    fn retain(&mut self, index: u32) {
        assert!(self.is_alive(index), "Retained object {index} is not alive");
        self.0[index as usize] += 1;
    }

    /// Remove one reference, and return whether the object ended with it.
    fn release(&mut self, index: u32) -> bool {
        assert!(self.is_alive(index), "Released object {index} is not alive");
        let count = &mut self.0[index as usize];
        *count -= 1;
        *count == 0
    }

    fn is_alive(&self, index: u32) -> bool {
        self.0.get(index as usize).is_some_and(|count| *count > 0)
    }
}

/// One value for each live object of one kind, indexed by the index of the object's id.
#[derive(Debug)]
struct Slots<T>(Vec<Option<T>>);

impl<T> Default for Slots<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T> Slots<T> {
    fn set(&mut self, index: u32, value: T) {
        let index = index as usize;
        if index >= self.0.len() {
            self.0.resize_with(index + 1, || None);
        }
        assert!(
            self.0[index].is_none(),
            "Object {index} already has a value"
        );
        self.0[index] = Some(value);
    }

    fn replace(&mut self, index: u32, value: T) -> T {
        self.0[index as usize]
            .replace(value)
            .unwrap_or_else(|| panic!("Object {index} has no value to replace"))
    }

    fn take(&mut self, index: u32) -> T {
        self.0[index as usize]
            .take()
            .unwrap_or_else(|| panic!("Object {index} has no value to take"))
    }
}

#[cfg(test)]
mod tests {
    use massive_geometry::Transform;

    use super::LifetimeStage;
    use crate::current_queue::test_queue::{take_changes, with_test_queue};
    use crate::prelude::identity_location;
    use crate::{Change, RenderChange, RenderSceneChange, SceneChange, id_generator};

    /// A location kept alive only by a reference still needs its transform, so dropping both
    /// handles must not end the transform. The transform ends together with the location, in the
    /// batch that releases the location's last reference.
    #[test]
    fn a_live_object_keeps_what_it_refers_to_until_it_ends() {
        with_test_queue(|| {
            let mut stage = LifetimeStage::default();
            let location = identity_location().submit_location().1.to_ref();

            let deleted = deletes(stage.resolve(take_changes()));
            assert!(deleted.is_empty(), "a live location lost: {deleted:?}");

            drop(location);
            let deleted = deletes(stage.resolve(take_changes()));
            assert_eq!(deleted, ["location", "transform"]);
        });
    }

    /// A location that keeps its transform under a new parent must not end the transform, even when
    /// nothing else holds it: the stage counts the new targets before it releases the previous ones.
    #[test]
    fn a_new_parent_keeps_the_transform_alive() {
        with_test_queue(|| {
            let mut stage = LifetimeStage::default();
            let (_parent_transform, parent) = identity_location().submit_location();
            let (transform, mut location) = identity_location().submit_location();
            drop(transform);
            stage.resolve(take_changes());

            location.set_parent(&parent);
            let deleted = deletes(stage.resolve(take_changes()));
            assert!(deleted.is_empty(), "reparenting ended: {deleted:?}");
        });
    }

    /// Submitting a location also references its transform. The transform's create must reach the
    /// queue before the location's, because the stage counts the reference when it sees the
    /// location's create.
    #[test]
    fn a_submitted_location_creates_its_transform_before_referencing_it() {
        with_test_queue(|| {
            let (_transform, _location) = identity_location().submit_location();
            LifetimeStage::default().resolve(take_changes());
        });
    }

    /// A reference keeps its object alive after the handle is dropped, and the object ends in the
    /// batch that releases its last reference (ADR 0040).
    #[test]
    fn an_object_ends_with_its_last_reference_not_its_handle() {
        let mut stage = LifetimeStage::default();
        let id = id_generator::acquire::<Transform>();

        let first = stage.resolve(vec![
            SceneChange::Transform(Change::Create(id, Transform::IDENTITY)),
            SceneChange::Transform(Change::Retain(id)),
            // The handle is dropped, the reference remains.
            SceneChange::Transform(Change::Release(id)),
        ]);
        assert!(matches!(
            first.as_slice(),
            [RenderSceneChange::Transform(RenderChange::Create(created, _))] if *created == id
        ));

        let last = stage.resolve(vec![SceneChange::Transform(Change::Release(id))]);
        assert!(matches!(
            last.as_slice(),
            [RenderSceneChange::Transform(RenderChange::Delete(deleted))] if *deleted == id
        ));
    }

    /// The kinds of the objects the render changes delete, in order.
    fn deletes(changes: Vec<RenderSceneChange>) -> Vec<&'static str> {
        changes
            .into_iter()
            .filter_map(|change| match change {
                RenderSceneChange::Transform(RenderChange::Delete(_)) => Some("transform"),
                RenderSceneChange::Location(RenderChange::Delete(_)) => Some("location"),
                RenderSceneChange::Visual(RenderChange::Delete(_)) => Some("visual"),
                _ => None,
            })
            .collect()
    }
}
