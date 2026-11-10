# A lifetime stage before the renderer decides when scene objects end

## Status: proposed

## Context

A scene object is a transform, a location or a visual that a task submits to the renderer. Today a
`Handle<T>` is an `Arc<InnerHandle<T>>` (`scene/src/handle.rs`). The inner handle holds the
object's id, the sink of the change queue the object was submitted to, and the value behind a
`Mutex`. Cloning a handle shares all three. A `Ref<T>` is the same `Arc`, restricted to reading.
When the last clone drops, the inner handle sends `Delete` through its stored sink. A `Ref` inside a
value, such as a visual's location, keeps the referenced object alive, so the renderer never sees a
reference to a deleted object. The shell releases the ids of deleted objects in `id_generator::gc`,
just before it applies a batch (`shell/src/window_renderer.rs`, `apply_scene_changes`).

The cost of this design is one heap allocation and one `Mutex` per object, plus an atomic counter on
every clone. The change queue pays as well. The task-local of ADR 0008 holds an `AnyCollector`,
whose two `Arc`s point at one `ChangeCollector`, which keeps its changes behind a
`parking_lot::Mutex` (`util/src/change_collector.rs`). The queue is shared this way only because
every handle stores a copy of its sink (`scene/src/scene.rs`). Through such a copy, another thread
can push into a task's queue, for example when an instance thread drops the last handle of a desktop
object. The goal of this decision is to remove the `Arc` from handles. `Rc` is no replacement,
because handles live on multi-threaded runtimes (`shell/src/shell.rs`, and
`RuntimeKind::MultiThread` in `desktop/src/instance_manager.rs`).

A compile experiment that removed `Clone` from `Handle` showed what the sharing is used for. Most
clones only keep a parent location alive. One clone in the launcher presenter was unnecessary. Only
the instance root had two writers. The instance presenter and its movement closure both updated
`layout_transform` and `layout_location`.

## Decision

### Handles and references

- `Handle<T>` is the single owner and the only writer of its object. It holds the id and the value
  inline, without a `Mutex`. Updates take `&mut self`. `value()` returns `&T`.
- `Handle<T>` is not `Clone`. In this design a clone would copy the value, so two clones would send
  competing snapshots of the whole object, and neither would see the other's writes. Code that needs
  to keep an object alive uses `Handle::to_ref`. Shared writing, if it is ever needed again, is
  spelled out as `Arc<Mutex<Handle<T>>>` at the site that needs it.
- `Ref<T>` is a counted reference that keeps its object alive. It carries only the id, so it cannot
  read the value. Application code reads values only through its own handles. The one reader
  through `Ref`s was `TransformResolver`, which a desktop test uses to resolve a launcher's
  transform. It now resolves from a batch of submitted scene changes instead, by walking their
  parent ids, which is also what the renderer receives. Because only that test needs it, it moves
  from `massive-scene` into the desktop's system tests.
- `Handle` keeps only `Debug`. It is not `PartialEq` or `Hash`, because a unique handle only equals
  itself. `Ref` stays `PartialEq` and `Hash` by id, because a location's parent compares through it.
- `Handle::update_if_changed_with` gives way to typed setters: `set_parent` and `set_alpha` on a
  location's handle, `set_shapes` and `set_clip_bounds` on a visual's handle. Every caller set
  exactly one field. Each setter compares that field with the new value and sends the update only
  when they differ, which needs no snapshot of the old value. A snapshot would clone the value's
  `Ref`s and so send a `Retain` and a `Release` for each.
- `InnerHandle`, `HandleValue`, `Ref::value` and the conversion `From<&Handle<T>> for Handle<T>` go
  away. `Visual` and `Location` lose `Clone`, which nothing needs, so no clone sends hidden
  `Retain`s.

### Changes

Handles send `Change<T>`, which has four variants.

- `Create(id, value)` starts the object's count at one, for its `Handle`.
- `Update(id, value)` replaces the value.
- `Retain(id)` is sent when a `Ref` is created or cloned.
- `Release(id)` is sent when a `Handle` or a `Ref` is dropped.

Handles never send `Delete`. The renderer receives a separate type with the three variants
`Create`, `Update` and `Delete`, and never sees `Retain` or `Release`. `SceneChange` exists once for
each side, so the compiler enforces both rules. Both kinds of change travel in the same queue.
`SceneChange::destructive_change` goes away together with `gc`.

### The lifetime stage

The lifetime stage decides when scene objects end. It runs in the shell, at the boundary between
the hosted instance's submission and the renderer, in place of `id_generator::gc`. Everything above
the shell only transports `Change<T>`. The stage takes each drained batch and does the following.

- It passes `Create` and `Update` on.
- It counts `Retain` and `Release` per object.
- It emits `Delete` when an object's count reaches zero, and returns the object's id to the id
  generator.

The stage keeps its counts in one plain vector per object kind, indexed by id, because ids are dense
per kind. Its code lives in `massive-scene`, next to the id generator it takes over. Each window
renderer has its own stage.

### Where a handle sends its changes

- A handle sends every change to the change queue of the task in which the change happens. It does
  not store a sink. A `Ref` dropped on an instance thread therefore sends its `Release` to that
  instance's queue, even when the object was submitted by the desktop.
- `massive-scene` reaches the current task's queue through a hook. `massive-applications` registers
  the hook before it installs any task context, and backs it with the task-local of ADR 0008. The
  choice of mechanism therefore stays in the layer that knows the runtimes. A later switch to
  thread-locals touches only that layer. The hook adds one indirect call to each clone, update and
  drop. That call is small compared to the task-local lookup, which the path already pays.
- `scene::submit(&collector, value)` and `submit_to(collector)` lose their explicit collector,
  because every later change of the object follows the hook anyway. Submitting always goes to the
  current task's queue.

### The task-local owns its change queue

Once handles no longer store a sink, only the owning task touches its change queue. The task pushes
through the hook, and its frame submission drains the queue through the same task-local. So the
task-local owns the queue directly, in a `RefCell`, without an `Arc` and without a `Mutex`.

- This holds on multi-threaded runtimes. Tokio keeps a task-local value inside the task's future and
  swaps it into a per-thread `RefCell<Option<T>>` for the duration of each poll
  (`tokio::task::task_local`, `LocalKey::scope_inner`). The value moves between threads with the
  task, but two threads never access it at the same time. A spawned future therefore needs the value
  to be `Send`, not `Sync`. The task-locals `ANIMATION` and `SHAPER` in
  `applications/src/task_context.rs` already hold a `RefCell`, and they run on the shell's
  multi-threaded runtime.
- The queue's type stays erased, because one task-local serves tasks whose change type is
  `SceneChange` and tasks whose change type is `InstanceChange` (ADR 0008). The erased queue sits in
  a `Box`, not behind an `Arc`.
- No code runs while the queue is borrowed. A push only converts and appends the change, and changes
  carry ids and render objects, not handles. Draining is a `mem::take`. So no drop can reach the
  queue while it is borrowed, and the `RefCell` never fails. Code that builds a value from a handle,
  such as `submit_pair`, does so before it borrows the queue. ADR 0008 rejected `RefCell` for
  borrows that cross an `await` or nest, and a push does neither.
- `AnyCollector::sink` and `task_context::sink` go away. The latter has no callers today.
- The lock between the shell's host task and the render thread (`RenderThreadSubmission` in
  `shell/src/window_renderer.rs`) stays. It hands submissions from one thread to another, so it is a
  real cross-thread queue.

With this, the path of a push no longer contains an atomic operation. It is the hook, the
task-local lookup and a `RefCell` borrow.

### Handles are used inside a task

A handle is created, cloned, updated and dropped only while a task context is installed. This is
enforced.

- The hook uses `try_with`. When no task context is installed, a `debug_assert!` fails, and release
  builds log an error. The change is lost, so the object stays in the renderer until the renderer
  ends. `Drop` must not panic, because a panic during unwinding aborts the process.
- `run_instance_future` in `desktop/src/instance_manager.rs` drops the instance future inside a task
  context when building the instance runtime fails. Today that future, which holds the instance
  root's `Ref<Location>`, drops on the instance thread without one.
- Tests that submit through an explicit collector install a task context instead. The tests of
  `massive-scene`, which sits below `massive-applications`, register a hook backed by a test queue.

### Call sites

- The clones that only keep a parent alive become `Ref<Location>`. These are in `ProjectPresenter`,
  `LauncherPresenter`, `project_matrix_location`, the title bar presenters, the root project's
  parent in `command_dispatch.rs`, and mt's `ScrollLocations`.
- The launcher presenter moves its transform handle into its movement closure instead of cloning it.
- `DesktopChange::SpawnInstance` carries `view_parent: Ref<Location>` instead of the whole
  `InstanceRoot`, because spawning only uses `InstanceRoot::view_parent`. `InstanceRoot` loses
  `Clone`.
- The instance root has a single writer per handle. The presenter writes the parent, the initial
  transform and the initial alpha in `InstancePresenter::new`, before it mounts the movement. Then it
  moves `layout_transform` and `layout_location` into the movement closure.
- `InstancePresenter::present_view` no longer sets the alpha to zero. The alpha is already zero,
  because construction sets it and the movement's `view_alpha` starts at zero. The `modify` that
  starts the fade-in also reaches the same submission, because `Frame::submission` runs the
  movement actions before it drains the queue.
- `InstancePresenter::latest_transform` returns the presenter's `target_transform`, because the
  presenter can no longer read the transform the closure wrote. The two differ only while the
  instance is still animating. Starting a new instance from it within the 500 ms of a layout
  animation lets the new instance appear at the originator's destination instead of its position on
  screen.

### Conditions this design relies on

- **Tasks form a tree.** Changes travel up only through submissions, and siblings do not
  communicate. A `Ref` crosses tasks only from a task to a task it starts, or inside a submission.
  Under this condition, the stage always sees a `Ref`'s `Retain` before its `Release`. The desktop,
  and the shell's host, push an instance's scene changes into their own queue in the order of the
  instance's output (`apply_instance_change` in `command_dispatch.rs`, `apply_change` in
  `instance_host.rs`). So the instance's earlier changes are queued before the desktop acts on a
  message from it. If a `Ref` ever reaches another task by any other path, its `Release` can
  overtake its `Retain`. The stage then deletes an object that is still referenced, and no error
  reports it.
- **A batch stays whole.** A dropped handle sends its own `Release`, and then the `Release`s of the
  references in its value, in one synchronous call. An update that replaces a reference sends the
  `Release` of the old target and the `Update` in one call. Each task drains its queue only in its
  own frame submission, so these changes land in the same batch. The stage must neither split a
  batch nor carry a `Delete` into the next one.
- **The renderer resolves references only between batches.** `SceneState::apply_changes` only
  writes table rows, and locations are resolved later, in `prepare`. So the order of changes inside
  a batch does not matter, only the state at its end. Drop order is not involved.
- **All changes of an object reach one renderer.** This holds today, because the shell draws one
  view into one renderer and the desktop draws into its one window.

## Out-of-process

The out-of-process plan (`docs/out-of-process-applications-plan.md`) moves applications into
their own processes, with scene changes as the boundary. This decision prepares for that in three
ways.

- **Reference counts are data.** Each instance gets a host-owned `view_parent: Ref<Location>`.
  Today that `Ref` is an `Arc` into the desktop's memory, which no other process can hold. Under this
  decision a `Ref` is an id, and its `Retain` and `Release` are changes that travel over a
  connection like any other change.
- **Handles hold no pointers.** An application process registers its own hook, so its
  `InstanceContext` works without changes to application code.
- **This decision must land before `scene::v1` is frozen.** The plan never edits a protocol version
  after release. `Retain` and `Release` belong in the scene protocol, and adding them later would
  need a `scene::v2`.

One gap remains. The lifetime stage returns ids to the id generator, and it runs only in front of
a renderer. An application process has no renderer, so the ids of its own objects would never be
released. The plan records this as an open question, together with its candidate answers and the
prior art. To keep every answer possible, the lifetime stage stays self-contained. It turns one
batch into another and depends on neither the shell nor the renderer, so an application process can
run it at its own edge.

## Considered Options

- **Arena handles.** The renderer side already is an arena. `Id` is a dense, recycled `u32`, and
  `IdTable` is a vector indexed by it. Copyable arena handles would give up the guarantee that a
  referenced object outlives its referrers, and no stale id can occur today.
- **Counts in the renderer, with uncounted `Ref`s.** A `Ref` held by client code but not yet placed
  in an object would not be counted. Once its owner dropped, the `Ref` would point at a deleted or
  reused id. Counting every `Ref` closes that gap, and then the counts need not live in the renderer.
- **Counts in each change queue.** This works, but every queue would carry a count table. The user
  preferred one place that decides lifetimes for all queues.
- **A stored sink in every handle.** Every change of an object would go to the queue it was
  submitted to, so the order of `Retain` and `Release` would hold even if siblings communicated, and
  a drop outside a task would be harmless. The cost is one shared `Arc` per handle. Within the tree
  of tasks the order holds without it, so the user chose handles without any `Arc`.
- **`Ref` as a borrow of its `Handle`.** The compiler would prove that a reference never outlives its
  object. But views own both a location and the visuals that refer to it, so they would become
  self-referential.
- **A separate queue for `Retain` and `Release`.** It keeps lifetime events out of the change
  stream. But every submission step would need a second path, and the queues would have to be
  drained together to keep a batch whole. The separation is only needed at the renderer, so separate
  types in one queue suffice.
- **The change-queue task-local inside `massive-scene`.** It avoids the hook's indirect call. But
  `massive-scene` would decide how a task is identified, and it would need tokio's `rt` feature.
- **A read path on `Movement` for `latest_transform`.** It would keep today's exact behavior, at the
  cost of a new read API in `massive-animation` for one caller.

## Consequences

- A `Ref` that is forgotten with `mem::forget`, or caught in a cycle, leaks its object. It never
  leaves a reference dangling.
- Building a value with a `Ref` sends a `Retain`. An animation that rebuilds a whole `Visual` every
  frame therefore sends a `Retain` and a `Release` per frame. The desktop's hover and zoom level
  indicator set only their shapes through `set_shapes` instead.
- A handle holds its value inline, so a type with many handles grows. `InstanceRoot` holds six, so
  the two enum variants that carry one, `DesktopCommand::StartInstance` and
  `DesktopChange::PresentInstance`, box their payload. Unboxed, every command and every change
  would be as large as these variants.
- Switching the hook to thread-locals later requires that every task holding handles runs on a
  current-thread runtime. Today the shell's runtime and instances with `RuntimeKind::MultiThread`
  move tasks between threads.
- When this is implemented, the glossary entries **Submit**, **Change sink** and **Change
  collector** change, because handles no longer hold the sink and the queue is no longer
  lock-guarded. The crate documentation of `massive-scene`, which describes
  reference-counted handles, changes as well.
