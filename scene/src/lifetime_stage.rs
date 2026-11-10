use massive_geometry::Transform;

use crate::id_generator;
use crate::{Change, Id, Location, Object, RenderChange, RenderSceneChange, SceneChange, Visual};

/// Decides when scene objects end (ADR 0040).
///
/// The stage sits between the queues that collect scene changes and the renderer. It keeps a
/// reference count for each object. The object's handle counts as one reference, every counted
/// [`Ref`](crate::Ref) as one more, and so does every live object that refers to it by id.
///
/// An object whose count reaches zero ends at the end of the batch, and only if its count is still
/// zero then. A later change in the same batch may refer to it again, so a value can take an id
/// from a reference that is dropped before the value is submitted, as long as both happen before
/// the task's next submission. When an object ends, the stage emits its delete, returns its id to
/// the id generator, and releases what the object referred to, which may end those objects in turn.
///
/// Each batch stays whole, its deletes included. The renderer relies on that, because it resolves
/// references only between batches.
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
            unreferenced: Vec::new(),
            ended: Vec::new(),
        };

        for change in changes {
            match change {
                SceneChange::Transform(change) => self.resolve_change(change, &mut resolution),
                SceneChange::Location(change) => self.resolve_change(change, &mut resolution),
                SceneChange::Visual(change) => self.resolve_change(change, &mut resolution),
            }
        }

        self.end_unreferenced(&mut resolution);
        id_generator::release_all(&resolution.ended);
        resolution.render_changes
    }

    fn resolve_change<T: Object>(&mut self, change: Change<T>, resolution: &mut Resolution) {
        match change {
            Change::Create(id, value) => {
                let targets = value.targets();
                self.retain_all(targets);
                let table = self.table(T::KIND);
                table.counts.create(id.index());
                table.targets.set(id.index(), targets);
                resolution.emit(T::render_change(RenderChange::Create(id, value)));
            }
            Change::Update(id, value) => {
                let targets = value.targets();
                let table = self.table(T::KIND);
                debug_assert!(
                    table.is_alive(id.index()),
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
                resolution.emit(T::render_change(RenderChange::Update(id, value)));
            }
            Change::Retain(id) => self.table(T::KIND).retain(id.index()),
            Change::Release(id) => self.release_all([Target::from(id)], resolution),
        }
    }

    fn retain_all(&mut self, targets: Targets) {
        for target in targets.iter() {
            self.table(target.kind).retain(target.index);
        }
    }

    /// Release references, and note every object whose count reaches zero, so the end of the batch
    /// can decide whether it ends.
    fn release_all(
        &mut self,
        targets: impl IntoIterator<Item = Target>,
        resolution: &mut Resolution,
    ) {
        for target in targets {
            if self.table(target.kind).counts.release(target.index) {
                resolution.unreferenced.push(target);
            }
        }
    }

    /// End every object whose count is still zero at the end of the batch, together with what only
    /// it kept alive.
    ///
    /// Nothing can refer to an object again once the batch has been read, so what an ended object
    /// referred to ends right away when its count reaches zero. A worklist instead of recursion,
    /// because a chain of locations can be arbitrarily long.
    fn end_unreferenced(&mut self, resolution: &mut Resolution) {
        let mut pending = std::mem::take(&mut resolution.unreferenced);
        while let Some(Target { kind, index }) = pending.pop() {
            let table = self.table(kind);
            // An object noted twice, or referred to again later in the batch, does not end here.
            if !table.is_alive(index) || table.counts.is_referenced(index) {
                continue;
            }
            for target in table.targets.take(index).iter() {
                if self.table(target.kind).counts.release(target.index) {
                    pending.push(target);
                }
            }
            resolution.end(kind, index);
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

/// The kind of a scene object, which selects its id space and its tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Transform,
    Location,
    Visual,
}

impl Kind {
    /// The position of this kind's id space and tables in arrays of all three.
    pub(crate) fn index(self) -> usize {
        self as usize
    }

    /// The delete the renderer receives for an ended object of this kind.
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
    pub(crate) kind: Kind,
    pub(crate) index: u32,
}

impl<T: Object> From<Id<T>> for Target {
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
    /// The targets of one object, built by its [`Object::targets`].
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
    /// What each live object refers to, from its latest create or update. An object is live from
    /// its create until it ends, even while its count is zero within a batch.
    targets: Slots<Targets>,
}

impl Table {
    /// Add one reference to a live object.
    fn retain(&mut self, index: u32) {
        assert!(self.is_alive(index), "Retained object {index} is not alive");
        self.counts.retain(index);
    }

    fn is_alive(&self, index: u32) -> bool {
        self.targets.is_set(index)
    }
}

/// What resolving one batch produces: the render changes, and the objects that ended.
#[derive(Debug)]
struct Resolution {
    render_changes: Vec<RenderSceneChange>,
    /// Objects whose count reached zero during the batch, which end at its end unless something
    /// referred to them again.
    unreferenced: Vec<Target>,
    ended: Vec<Target>,
}

impl Resolution {
    fn emit(&mut self, change: RenderSceneChange) {
        self.render_changes.push(change);
    }

    /// Record an ended object: its delete goes to the renderer, and its id back to the generator.
    fn end(&mut self, kind: Kind, index: u32) {
        self.ended.push(Target { kind, index });
        self.render_changes.push(kind.delete(index));
    }
}

/// The reference count of each object of one kind, indexed by the index of the object's id.
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
        self.0[index as usize] += 1;
    }

    /// Remove one reference, and return whether it was the last one.
    fn release(&mut self, index: u32) -> bool {
        assert!(
            self.is_referenced(index),
            "Released object {index} has no references"
        );
        let count = &mut self.0[index as usize];
        *count -= 1;
        *count == 0
    }

    fn is_referenced(&self, index: u32) -> bool {
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

    fn is_set(&self, index: u32) -> bool {
        self.0.get(index as usize).is_some_and(Option::is_some)
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
    use crate::prelude::{Submit, identity_location};
    use crate::{Change, RenderChange, RenderSceneChange, SceneChange, id_generator};

    /// A location kept alive only by a reference still needs its transform, so dropping both
    /// handles must not end the transform. The transform ends together with the location, in the
    /// batch that releases the location's last reference.
    #[test]
    fn a_live_object_keeps_what_it_refers_to_until_it_ends() {
        with_test_queue(|| {
            let mut stage = LifetimeStage::default();
            let location = identity_location().submit().1.to_ref();

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
            let (_parent_transform, parent) = identity_location().submit();
            let (transform, mut location) = identity_location().submit();
            drop(transform);
            stage.resolve(take_changes());

            location.set_parent(&parent);
            let deleted = deletes(stage.resolve(take_changes()));
            assert!(deleted.is_empty(), "reparenting ended: {deleted:?}");
        });
    }

    /// A value may take an id from a reference that is dropped before the value is submitted. The
    /// object stays alive, because an object ends only if its count is still zero at the end of the
    /// batch.
    #[test]
    fn a_reference_dropped_before_its_referrer_is_submitted_keeps_the_object() {
        with_test_queue(|| {
            let mut stage = LifetimeStage::default();
            let parent = identity_location().submit().1.to_ref();
            stage.resolve(take_changes());

            let child = identity_location().relative_to(&parent);
            drop(parent);
            let (_transform, _location) = child.submit();
            let deleted = deletes(stage.resolve(take_changes()));
            assert!(deleted.is_empty(), "the parent ended: {deleted:?}");
        });
    }

    /// Submitting a location also references its transform. The transform's create must reach the
    /// queue before the location's, because the stage counts the reference when it sees the
    /// location's create.
    #[test]
    fn a_submitted_location_creates_its_transform_before_referencing_it() {
        with_test_queue(|| {
            let (_transform, _location) = identity_location().submit();
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
