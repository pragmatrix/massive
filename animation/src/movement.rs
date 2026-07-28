use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::mem;
use std::ptr;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::AnimationContext;

pub trait ApplyAnimations {
    fn apply_animations(&mut self, coordinator: &mut dyn AnimationContext);
}

pub struct Movements {
    active: HashMap<*const (), Box<dyn ActiveMovement>>,
    queue: Arc<Mutex<Vec<MovementAction>>>,
    // Reused while draining the queue so recurring actions retain their allocation capacity.
    actions: Vec<MovementAction>,
}

impl fmt::Debug for Movements {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Movements")
            .field("active_count", &self.active.len())
            .finish()
    }
}

impl Default for Movements {
    fn default() -> Self {
        Self::new()
    }
}

impl Movements {
    pub fn new() -> Self {
        Self {
            active: Default::default(),
            queue: Arc::new(Mutex::new(Vec::new())),
            actions: Vec::new(),
        }
    }

    #[must_use]
    pub fn add<T>(&mut self, value: T) -> MovementReference<T>
    where
        T: Any + Send + ApplyAnimations,
    {
        let value = Box::new(value);
        let reference = MovementReference::new(value.as_ref(), self.queue.clone());
        self.active.insert(reference.instance, value);

        reference
    }

    pub fn run_actions(&mut self) {
        mem::swap(&mut self.actions, &mut *self.queue.lock());

        for action in self.actions.drain(..) {
            match action {
                MovementAction::Drop(pointer) => {
                    self.active.remove(&pointer);
                }
                MovementAction::Modify(pointer, apply) => {
                    if let Some(instance) = self.active.get_mut(&pointer) {
                        apply(instance.as_any_mut());
                    }
                }
            }
        }
    }

    pub fn apply_animations(&mut self, context: &mut impl AnimationContext) {
        for movement in self.active.values_mut() {
            movement.apply_animations(context);
        }
    }
}

#[derive(Debug)]
pub struct MovementReference<T> {
    instance: *const (),
    queue: Arc<Mutex<Vec<MovementAction>>>,
    marker: PhantomData<fn(T)>,
}

impl<T> MovementReference<T> {
    fn new(instance: &T, queue: Arc<Mutex<Vec<MovementAction>>>) -> Self {
        Self {
            instance: ptr::from_ref(instance).cast(),
            queue,
            marker: PhantomData,
        }
    }

    pub fn modify(&self, modifier: impl FnOnce(&mut T) + Send + 'static)
    where
        T: Any + Send,
    {
        self.queue.lock().push(MovementAction::Modify(
            self.instance,
            Box::new(move |value| {
                let value = value
                    .downcast_mut::<T>()
                    .expect("movement reference has the wrong value type");
                modifier(value);
            }),
        ));
    }
}

// The pointer is an opaque identifier; it is only dereferenced by the owner thread through `active`.
unsafe impl<T> Send for MovementReference<T> {}

impl<T> Drop for MovementReference<T> {
    fn drop(&mut self) {
        self.queue.lock().push(MovementAction::Drop(self.instance));
    }
}

trait ActiveMovement: Any + Send + ApplyAnimations {
    fn as_any_mut(&mut self) -> &mut (dyn Any + Send);
}

impl<T: Any + Send + ApplyAnimations> ActiveMovement for T {
    fn as_any_mut(&mut self) -> &mut (dyn Any + Send) {
        self
    }
}

type ModifyMovement = Box<dyn FnOnce(&mut (dyn Any + Send)) + Send>;

enum MovementAction {
    Drop(*const ()),
    Modify(*const (), ModifyMovement),
}

// The pointer is an opaque identifier; it is only used by `Movements::run_actions` for lookup.
unsafe impl Send for MovementAction {}

impl fmt::Debug for MovementAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Drop(_) => formatter.write_str("Drop"),
            Self::Modify(_, _) => formatter.write_str("Modify"),
        }
    }
}
