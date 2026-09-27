//! The owning ambient shaping handle: the task's [`ShapingContext`] moved out of the task-local
//! for as long as the handle is open, then restored on drop.
//!
//! The context is owned because the task-local borrow cannot escape the access closure, while the
//! context has to stay reachable across a whole batch of shapes (ADR 0008). Moving it out and
//! restoring it on drop is what keeps the scratch exclusive to the task without a closure.
//!
//! Only shaping opens a handle. A pure manager read goes through
//! [`fonts()`](crate::task_context::fonts), which reads the installed [`ShapingContext`] in place.
//!
//! The context's scratch is not held for the handle's whole life: each `shape`/`glyph_run`
//! opens a short-lived session over it (`ShapingContext::shaper` borrows the scratch mutably),
//! shapes through it, and drops it. The session cannot be stored on the handle because it borrows
//! the context it came from; exclusivity against a second handle comes from the context being taken
//! out of the task-local.

use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

use massive_shapes::{
    FaceId, FaceMetrics, FontData, FontManager, FontRegistry, GlyphRun, ShapedRun, ShapingContext,
    ShapingEngineKind, ShapingRequest, ShapingSession,
};

use super::SHAPER;

/// The task's shaping context, owned for as long as the handle is open.
pub struct Shaper {
    /// `Option` because `Drop::drop(&mut self)` must move the context back out to restore it;
    /// a plain field cannot be moved out of `&mut self`.
    context: Option<ShapingContext>,
    /// Captured when the handle opens and refreshed after each shape, so the borrowed reads
    /// (`metrics`/`registry`/`font_data`) can return references into it — a per-call session is a
    /// temporary and cannot be borrowed from. Mirrors `massive_shapes::Shaper`'s own `registry`.
    registry: Arc<FontRegistry>,
    /// The only thing making the handle `!Send`. The handle reaches the task-local: moving it to
    /// another task would make `Drop` fail to restore the context there, stranding the originating
    /// task's slot (a later, misattributed panic). Deliberate — do not replace with `Send`.
    _not_send: PhantomData<Rc<()>>,
}

impl std::fmt::Debug for Shaper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shaper")
            .field("engine", &self.context_ref().manager().engine_kind())
            .finish_non_exhaustive()
    }
}

impl Shaper {
    /// The font manager the task's shaping owner shapes for.
    pub fn manager(&self) -> FontManager {
        self.context_ref().manager()
    }

    fn context_ref(&self) -> &ShapingContext {
        self.context
            .as_ref()
            .expect("Shaper owns its context until it is dropped")
    }

    fn context_mut(&mut self) -> &mut ShapingContext {
        self.context
            .as_mut()
            .expect("Shaper owns its context until it is dropped")
    }
}

impl Drop for Shaper {
    fn drop(&mut self) {
        // `try_with` so a handle dropped after its scope exited drops the context quietly instead
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
                         handle was open"
                    );
                } else {
                    // Outside unwinding the breach must fail hard; `context` is dropped here,
                    // which is acceptable because the breach is fatal.
                    panic!(
                        "shaping context lost: the task-local slot was refilled while a Shaper \
                         handle was open"
                    );
                }
            });
        }
    }
}

impl ShapingSession for Shaper {
    fn shape(&mut self, request: &ShapingRequest<'_>, font_size: f32) -> Option<ShapedRun> {
        let run = self.context_mut().shaper().shape(request, font_size);
        // Faces resolved during the shape are published; refresh so this handle's borrowed reads
        // see them (mirrors `massive_shapes::Shaper::shape`).
        self.registry = self.context_ref().manager().published();
        run
    }

    fn glyph_run(&mut self, request: &ShapingRequest<'_>, font_size: f32) -> Option<GlyphRun> {
        let run = self.context_mut().shaper().glyph_run(request, font_size);
        self.registry = self.context_ref().manager().published();
        run
    }

    fn font_data(&self, id: FaceId) -> Option<FontData> {
        self.registry.font_data(id)
    }

    fn metrics(&self, id: FaceId) -> Option<&FaceMetrics> {
        self.registry.metrics(id)
    }

    fn registry(&self) -> &FontRegistry {
        &self.registry
    }

    fn engine_kind(&self) -> ShapingEngineKind {
        self.context_ref().manager().engine_kind()
    }
}

/// Move the task's shaping context out into an owning handle, restoring it on drop.
///
/// The handle is short-lived by contract: it owns the task's context (and with it the exclusive
/// scratch), so two open handles panic rather than alias (ADR 0006, ADR 0008).
pub fn shaper() -> Shaper {
    SHAPER
        .try_with(|slot| {
            let context = slot.borrow_mut().take().unwrap_or_else(|| {
                panic!(
                    "shaping reentrancy: the task-local slot is empty because a Shaper handle is \
                     already open in this task; drop the handle before opening another"
                )
            });
            let registry = context.manager().published();
            Shaper {
                context: Some(context),
                registry,
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

    use static_assertions::{assert_impl_all, assert_not_impl_any};

    use massive_animation::{AnimationCoordinator, MovementRuntime};
    use massive_scene::{AnyCollector, SceneChange};
    use massive_shapes::{ShapingEngineKind, ShapingRequest, TextAttributes};

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
    async fn a_second_open_handle_panics() {
        with_context(contexts(), async {
            let _first = shaper();
            let _second = shaper();
        })
        .await;
    }

    #[tokio::test]
    async fn drop_restores_the_context_for_the_next_handle() {
        with_context(contexts(), async {
            fonts()
                .load_font(JETBRAINS_MONO)
                .expect("bundled font is valid");

            let mut handle = shaper();
            let first = handle
                .shape(&request("a"), 16.0)
                .expect("the open handle must shape");
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
    async fn a_handle_dropped_after_its_scope_exited_does_not_panic() {
        let handle = with_context(contexts(), async { shaper() }).await;
        // The task-local scope is gone; the restore must be a quiet no-op, not a panic.
        drop(handle);
    }

    // Pins the hard-fail invariant: if something refills the task-local slot while a handle holds
    // the taken context, `Drop` must not silently discard the owned context (which would strand
    // the task with the wrong context and a later, misleading reentrancy panic). It panics.
    #[tokio::test]
    #[should_panic(expected = "shaping context lost")]
    async fn refilling_the_slot_while_a_handle_is_open_panics_on_drop() {
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
    async fn fonts_panics_while_a_handle_is_open_and_recovers_after_drop() {
        with_context(contexts(), async {
            let handle = shaper();

            // The context is checked out for the open handle, so `fonts()` cannot read the
            // manager out of the slot while shaping.
            let result = std::panic::catch_unwind(fonts);
            let payload = match result {
                Err(payload) => payload,
                Ok(_) => panic!("fonts() must panic while a handle is open"),
            };
            let message = payload
                .downcast_ref::<&str>()
                .expect("panic payload is the static message");
            assert!(message.contains("fonts()"));

            // Dropping the handle restores the context, so `fonts()` must work again.
            drop(handle);
            let _recovered = fonts();
        })
        .await;
    }

    #[tokio::test]
    async fn shaping_refreshes_the_handles_registry_reads() {
        with_context(contexts(), async {
            let mut handle = shaper();
            // Load the font *after* the handle opened, so its open-time snapshot predates this
            // face; only the post-shape refresh can make the borrowed reads resolve it.
            handle
                .manager()
                .load_font(JETBRAINS_MONO)
                .expect("bundled font is valid");

            let run = handle
                .shape(&request("a"), 16.0)
                .expect("the open handle must shape");
            let face = run.glyphs[0].face_id;

            assert!(handle.font_data(face).is_some());
            assert!(handle.metrics(face).is_some());
        })
        .await;
    }

    // The handle is a batch session: scratch is opened per `shape` call and released, so a second
    // shape through the same handle must succeed.
    #[tokio::test]
    async fn one_handle_shapes_a_batch() {
        with_context(contexts(), async {
            fonts()
                .load_font(JETBRAINS_MONO)
                .expect("bundled font is valid");

            let mut handle = shaper();
            let first = handle
                .shape(&request("a"), 16.0)
                .expect("first shape through the handle");
            let second = handle
                .shape(&request("bb"), 16.0)
                .expect("second shape through the handle");
            assert!(!first.glyphs.is_empty());
            assert!(!second.glyphs.is_empty());
        })
        .await;
    }

    // A handle that reached another task would strand the originating task's slot, so the handle
    // must stay on its task.
    assert_not_impl_any!(Shaper: Send);
    // The borrowed session's only non-`Send` borrow is the `&mut Box<dyn EngineScratch>`
    // (the `&ShapingContext` is `Send`): a session is movable, not shareable, and is dropped
    // before the frame it shaped for is submitted.
    assert_impl_all!(massive_shapes::Shaper<'static>: Send);
    assert_not_impl_any!(massive_shapes::Shaper<'static>: Sync);
}
