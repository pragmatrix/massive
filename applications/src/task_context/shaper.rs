//! The task's shaping guard: the task's [`ShapingContext`] moved out of the task-local for as long
//! as the guard is open, then restored on drop.
//!
//! The context is owned because the task-local borrow cannot escape the access closure, while the
//! context has to stay reachable across a whole batch of shapes (ADR 0008). Moving it out and
//! restoring it on drop is what keeps the scratch exclusive to the task without a closure; the
//! guard lends the context through `Deref`, so call sites shape through `&mut shaper` exactly as
//! they would through a context they own.
//!
//! Only shaping opens a guard. A pure manager read goes through
//! [`fonts()`](crate::task_context::fonts), which reads the installed [`ShapingContext`] in place.

use std::fmt;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use massive_shapes::ShapingContext;

use super::SHAPER;

/// The task's shaping context, held for as long as the guard is open.
pub struct Shaper {
    /// `Option` because `Drop::drop(&mut self)` must move the context back out to restore it;
    /// a plain field cannot be moved out of `&mut self`.
    context: Option<ShapingContext>,
    /// The only thing making the guard `!Send`. The guard reaches the task-local: moving it to
    /// another task would make `Drop` fail to restore the context there, stranding the originating
    /// task's slot (a later, misattributed panic). Deliberate — do not replace with `Send`.
    _not_send: PhantomData<Rc<()>>,
}

impl fmt::Debug for Shaper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shaper")
            .field("engine", &self.engine_kind())
            .finish_non_exhaustive()
    }
}

impl Deref for Shaper {
    type Target = ShapingContext;

    fn deref(&self) -> &ShapingContext {
        self.context
            .as_ref()
            .expect("Shaper holds its context until it is dropped")
    }
}

impl DerefMut for Shaper {
    fn deref_mut(&mut self) -> &mut ShapingContext {
        self.context
            .as_mut()
            .expect("Shaper holds its context until it is dropped")
    }
}

impl Drop for Shaper {
    fn drop(&mut self) {
        // `try_with` so a guard dropped after its scope exited drops the context quietly instead
        // of panicking (this also keeps it quiet during unwinding).
        if let Some(context) = self.context.take() {
            let _ = SHAPER.try_with(|slot| {
                let mut slot = slot.borrow_mut();
                if slot.is_none() {
                    *slot = Some(context);
                } else if std::thread::panicking() {
                    // A refilled slot means the task's context would be silently discarded here,
                    // and the next `shaper()` would panic with a misleading reentrancy message.
                    // This is a real invariant breach, but panicking while unwinding would abort
                    // the process, so report it without failing.
                    log::error!(
                        "shaping context lost: the task-local slot was refilled while a Shaper \
                         guard was open"
                    );
                } else {
                    // Outside unwinding the breach must fail hard; `context` is dropped here,
                    // which is acceptable because the breach is fatal.
                    panic!(
                        "shaping context lost: the task-local slot was refilled while a Shaper \
                         guard was open"
                    );
                }
            });
        }
    }
}

/// Move the task's shaping context out into an owning guard, restoring it on drop.
///
/// The guard is short-lived by contract: it owns the task's context (and with it the exclusive
/// scratch), so two open guards panic rather than alias (ADR 0006, ADR 0008).
pub fn shaper() -> Shaper {
    SHAPER
        .try_with(|slot| {
            let mut context = slot.borrow_mut().take().unwrap_or_else(|| {
                panic!(
                    "shaping reentrancy: the task-local slot is empty because a Shaper guard is \
                     already open in this task; drop the guard before opening another"
                )
            });
            // Open-time sync and snapshot: a read taken before the batch's first shape must see the
            // world as it is now, not as the context last saw it.
            context.refresh();
            Shaper {
                context: Some(context),
                _not_send: PhantomData,
            }
        })
        .unwrap_or_else(|_| {
            panic!(
                "no shaping context installed: a shaper requires the task context, so call \
                 task_context::with_context before opening one"
            )
        })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use static_assertions::assert_not_impl_any;

    use massive_animation::{AnimationCoordinator, MovementRuntime};
    use massive_scene::{AnyCollector, SceneChange};
    use massive_shapes::{FontManager, ShapingEngineKind, ShapingRequest, TextAttributes};

    use super::*;
    use crate::task_context::{TaskContext, fonts, with_context};

    const JETBRAINS_MONO: &[u8] = include_bytes!(
        "../../../assets/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf"
    );

    fn shaping_context() -> ShapingContext {
        FontManager::bare(ShapingEngineKind::available()[0]).new_shaping_context()
    }

    fn contexts() -> TaskContext {
        TaskContext::new(
            AnyCollector::for_type::<SceneChange>(),
            AnimationCoordinator::new(),
            MovementRuntime::default(),
            shaping_context(),
        )
    }

    fn request(text: &'static str) -> ShapingRequest<'static> {
        ShapingRequest::new(text, TextAttributes::default())
    }

    #[test]
    #[should_panic(expected = "no shaping context installed")]
    fn opening_without_a_context_panics_with_the_install_point() {
        shaper();
    }

    #[tokio::test]
    #[should_panic(expected = "shaping reentrancy")]
    async fn a_second_open_guard_panics() {
        with_context(contexts(), async {
            let _first = shaper();
            let _second = shaper();
        })
        .await;
    }

    #[tokio::test]
    async fn drop_restores_the_context_for_the_next_guard() {
        with_context(contexts(), async {
            fonts()
                .load_font(JETBRAINS_MONO)
                .expect("bundled font is valid");

            let mut handle = shaper();
            let first = handle
                .shape(&request("a"), 16.0)
                .expect("the open guard must shape");
            assert!(!first.glyphs.is_empty());
            let face = first.glyphs[0].face_id;
            drop(handle);

            // Reopening only succeeds if `Drop` put the context back; otherwise the slot is still
            // `None` and this panics with the reentrancy message.
            let mut reopened = shaper();
            assert!(reopened.shape(&request("a"), 16.0).is_some());
            assert!(reopened.font_data(face).is_some());
        })
        .await;
    }

    #[tokio::test]
    async fn a_guard_dropped_after_its_scope_exited_does_not_panic() {
        let handle = with_context(contexts(), async { shaper() }).await;
        // The task-local scope is gone; the restore must be a quiet no-op, not a panic.
        drop(handle);
    }

    // Pins the hard-fail invariant: if something refills the task-local slot while a guard holds
    // the taken context, `Drop` must not silently discard the owned context (which would strand
    // the task with the wrong context and a later, misleading reentrancy panic). It panics.
    #[tokio::test]
    #[should_panic(expected = "shaping context lost")]
    async fn refilling_the_slot_while_a_guard_is_open_panics_on_drop() {
        // Write into the same task-local directly; nesting `with_context` would shadow the local,
        // so it could not reproduce the occupied-slot breach.
        SHAPER
            .scope(RefCell::new(Some(shaping_context())), async {
                let handle = shaper();
                let other = shaping_context();
                SHAPER.with(|slot| *slot.borrow_mut() = Some(other));
                drop(handle);
            })
            .await;
    }

    #[tokio::test]
    async fn fonts_panics_while_a_guard_is_open_and_recovers_after_drop() {
        with_context(contexts(), async {
            let handle = shaper();

            // The context is checked out for the open guard, so `fonts()` cannot read the
            // manager out of the slot while shaping.
            let result = std::panic::catch_unwind(fonts);
            let payload = match result {
                Err(payload) => payload,
                Ok(_) => panic!("fonts() must panic while a guard is open"),
            };
            let message = payload
                .downcast_ref::<&str>()
                .expect("panic payload is the static message");
            assert!(message.contains("fonts()"));

            // Dropping the guard restores the context, so `fonts()` must work again.
            drop(handle);
            let _recovered = fonts();
        })
        .await;
    }

    #[tokio::test]
    async fn a_batch_sees_a_font_loaded_after_the_guard_opened() {
        with_context(contexts(), async {
            let mut handle = shaper();
            // Loading a font after the guard opened moves the published face world; the guard's
            // post-shape refresh is what makes the loaded face resolvable through the guard.
            handle
                .manager()
                .load_font(JETBRAINS_MONO)
                .expect("bundled font is valid");

            let run = handle
                .shape(&request("a"), 16.0)
                .expect("the open guard must shape");
            let face = run.glyphs[0].face_id;

            assert!(handle.font_data(face).is_some());
            assert!(handle.metrics(face).is_some());
        })
        .await;
    }

    // The guard is a batch surface: one guard shapes many clusters through one context.
    #[tokio::test]
    async fn one_guard_shapes_a_batch() {
        with_context(contexts(), async {
            fonts()
                .load_font(JETBRAINS_MONO)
                .expect("bundled font is valid");

            let mut handle = shaper();
            let first = handle
                .shape(&request("a"), 16.0)
                .expect("first shape through the guard");
            let second = handle
                .shape(&request("bb"), 16.0)
                .expect("second shape through the guard");
            assert!(!first.glyphs.is_empty());
            assert!(!second.glyphs.is_empty());
        })
        .await;
    }

    // A guard that reached another task would strand the originating task's slot, so it must stay
    // on its task.
    assert_not_impl_any!(Shaper: Send);
}
