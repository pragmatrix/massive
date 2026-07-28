use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::mem;
use std::ptr;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::AnimationContext;

pub struct MovementRuntime {
    active: HashMap<MovementReference, Box<dyn ActiveMovement>>,
    queue: Arc<Mutex<Vec<MovementAction>>>,
    // Reused while draining the queue so recurring actions retain their allocation capacity.
    actions: Vec<MovementAction>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
struct MovementReference(*const ());

// This is an opaque identity only; movement values are never accessed through the pointer.
unsafe impl Send for MovementReference {}
unsafe impl Sync for MovementReference {}

impl MovementReference {
    fn new<T>(value: &T) -> Self {
        Self(ptr::from_ref(value).cast())
    }
}

impl fmt::Debug for MovementRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Movements")
            .field("active_count", &self.active.len())
            .finish()
    }
}

impl Default for MovementRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl MovementRuntime {
    pub fn new() -> Self {
        Self {
            active: Default::default(),
            queue: Arc::new(Mutex::new(Vec::new())),
            actions: Vec::new(),
        }
    }

    #[must_use]
    pub fn add<T, F>(&mut self, value: T, apply_animations: F) -> Movement<T>
    where
        T: Any + Send + Sync,
        F: FnMut(&mut T, &mut dyn AnimationContext) + Send + Sync + 'static,
    {
        let active = Box::new(ActiveMovementValue {
            apply_animations,
            value,
        });
        let reference = Movement::new(&active.value, self.queue.clone());
        self.active.insert(reference.instance, active);

        reference
    }

    pub fn run_actions(&mut self, context: &mut dyn AnimationContext) {
        mem::swap(&mut self.actions, &mut *self.queue.lock());

        for action in self.actions.drain(..) {
            match action {
                MovementAction::Drop(pointer) => {
                    self.active.remove(&pointer);
                }
                MovementAction::Modify(pointer, apply) => {
                    if let Some(instance) = self.active.get_mut(&pointer) {
                        apply(instance.as_any_mut(), context);
                    }
                }
            }
        }
    }

    pub fn apply_animations(&mut self, context: &mut dyn AnimationContext) {
        for movement in self.active.values_mut() {
            movement.apply_animations(context);
        }
    }
}

#[derive(Debug)]
pub struct Movement<T> {
    instance: MovementReference,
    queue: Arc<Mutex<Vec<MovementAction>>>,
    marker: PhantomData<fn(T)>,
}

impl<T> Movement<T> {
    fn new(instance: &T, queue: Arc<Mutex<Vec<MovementAction>>>) -> Self {
        Self {
            instance: MovementReference::new(instance),
            queue,
            marker: PhantomData,
        }
    }

    pub fn modify(
        &self,
        modifier: impl FnOnce(&mut T, &mut dyn AnimationContext) + Send + Sync + 'static,
    ) where
        T: Any + Send + Sync,
    {
        self.queue.lock().push(MovementAction::Modify(
            self.instance,
            Box::new(move |value, context| {
                let value = value
                    .downcast_mut::<T>()
                    .expect("movement reference has the wrong value type");
                modifier(value, context);
            }),
        ));
    }
}

impl<T> Drop for Movement<T> {
    fn drop(&mut self) {
        self.queue.lock().push(MovementAction::Drop(self.instance));
    }
}

trait ActiveMovement: Any + Send + Sync {
    fn as_any_mut(&mut self) -> &mut (dyn Any + Send);

    fn apply_animations(&mut self, context: &mut dyn AnimationContext);
}

struct ActiveMovementValue<T, F> {
    apply_animations: F,
    value: T,
}

impl<T, F> ActiveMovement for ActiveMovementValue<T, F>
where
    T: Any + Send + Sync,
    F: FnMut(&mut T, &mut dyn AnimationContext) + Send + Sync + 'static,
{
    fn as_any_mut(&mut self) -> &mut (dyn Any + Send) {
        &mut self.value
    }

    fn apply_animations(&mut self, context: &mut dyn AnimationContext) {
        (self.apply_animations)(&mut self.value, context);
    }
}

type ModifyMovement =
    Box<dyn FnOnce(&mut (dyn Any + Send), &mut dyn AnimationContext) + Send + Sync>;

enum MovementAction {
    Drop(MovementReference),
    Modify(MovementReference, ModifyMovement),
}

impl fmt::Debug for MovementAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Drop(_) => formatter.write_str("Drop"),
            Self::Modify(_, _) => formatter.write_str("Modify"),
        }
    }
}
