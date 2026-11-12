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

### Handles and ids

- `Handle<T>` is the single owner and the only writer of its object. It holds the id and the value
  inline, without a `Mutex`. Updates take `&mut self`. `value()` returns `&T`.
- `Handle<T>` is not `Clone`. In this design a clone would copy the value, so two clones would send
  competing snapshots of the whole object, and neither would see the other's writes.
- There is no counted reference. An object lives as long as its handle, and after that as long as
  live objects refer to it. Code that only places something relative to an object passes the
  object's `Id`. Code that has to keep an object alive owns its handle. An instance's view parent
  shows how this works across tasks: the desktop submits the location with its parent already set
  and moves the handle into the instance, which owns it from then on.
- Application code reads values only through its own handles. The one reader through shared handles
  was `TransformResolver`, which a desktop test uses to resolve a launcher's transform. It now
  resolves from submitted scene changes instead, by walking their parent ids, which is also what the
  renderer receives. Because only that test needs it, it moves from `massive-scene` into the
  desktop's system tests.
- Ids are typed. `Id<T>` names an object of type `T`, because every object type has its own id
  space. Handles, changes, values and the renderer's tables all use it, so the
  compiler keeps one type's ids out of another type's table. Only the lifetime stage, whose
  worklist mixes kinds, and the id generator keep a raw index, privately.
- A value refers to other objects by `Id<T>`, which does not count. A visual names its location,
  and a location its transform and its parent, this way. The lifetime stage counts
  these references itself, for as long as the referring object lives. A counted reference inside a
  value would not do. The value lives in the handle, so dropping the handle would release the
  value's references while other referrers still kept the object alive. The object would then
  outlive its own transform or parent, which mt showed as content without its padding transform.
- The values are the render objects. `Visual`, `Location` and `Transform` travel to the renderer as
  they are, and `VisualRenderObj`, `LocationRenderObj` and `LocationParentId` go away. The `Object`
  trait loses its associated change type. It is sealed, and it names the kind's index, the
  objects its value refers to, and the conversions of its changes into `SceneChange` and the
  renderer's change type. The lifetime stage resolves the three kinds through it with one generic
  function.
- The kinds' indices and their number, `OBJECT_KINDS`, are defined together, in one module of
  `massive-scene` that is not part of its API. The id generator and the lifetime stage keep one
  entry per kind in arrays of that length, instead of a map keyed by `TypeId`. Where objects of
  every kind are handled together, such as in the stage's worklist, they are named by a
  `TaggedId`, the index of an id tagged with its kind's index. The one step that needs the type
  back, the renderer's typed delete, looks it up in a table of one function per kind, defined next
  to the indices.
- `Handle` keeps only `Debug`. It is not `PartialEq` or `Hash`, because a unique handle only equals
  itself.
- `Handle::update_if_changed_with` gives way to typed setters: `set_parent` and `set_alpha` on a
  location's handle, `set_shapes` and `set_clip_bounds` on a visual's handle. Every caller set
  exactly one field. Each setter compares that field with the new value and sends the update only
  when they differ, which needs no snapshot of the old value.
- `InnerHandle`, `HandleValue`, `Ref` and the conversion `From<&Handle<T>> for Handle<T>` go away.

### Changes

Handles send `Change<T>`, which has three variants.

- `Create(id, value)` starts the object's count at one, for its `Handle`.
- `Update(id, value)` replaces the value.
- `Release(id)` is sent when the `Handle` is dropped.

Handles never send `Delete`. The renderer receives a separate type with the three variants
`Create`, `Update` and `Delete`, and never sees `Release`. `SceneChange` exists once for
each side, so the compiler enforces both rules. Both kinds of change travel in the same queue.
`SceneChange::destructive_change` goes away together with `gc`.

### The lifetime stage

The lifetime stage decides when scene objects end. It runs in the shell, at the boundary between
the hosted instance's submission and the renderer, in place of `id_generator::gc`. Everything above
the shell only transports `Change<T>`. The stage takes each scene change batch, the scene changes a
window renderer takes at once, and does the following.

- It passes `Create` and `Update` on.
- It counts each object's `Release`.
- It counts one reference for every object that a created location or visual refers to, and
  records those references. An update that changes the references counts the new ones and releases
  the previous ones. Their order does not matter, because an object ends only at the end of the
  scene change batch. An update that keeps its references leaves the counts alone.
- It notes every object whose count reaches zero. At the end of the scene change batch, it ends each
  noted object whose count is still zero. It emits the object's `Delete`, returns its id to the id
  generator, and releases what the object referred to, which may end those objects, too. An object
  that a later change in the same scene change batch refers to again stays alive, so a value may
  take an id from a handle that is dropped before the value is submitted.

The stage keeps its counts, and the references of live locations and visuals, in one plain vector
per object kind, indexed by id, because ids are dense per kind. Its code lives in `massive-scene`,
next to the id generator it takes over. Each window renderer has its own stage.

### Where a handle sends its changes

- A handle sends every change to the change queue of the task in which the change happens. It does
  not store a sink. A handle dropped on an instance thread therefore sends its `Release` to that
  instance's queue, even when the desktop submitted the object, as it does for the view parent.
- `massive-scene` reaches the current task's queue through a hook, a plain function
  (`CollectChange`) that collects one change and reports whether a queue was reached.
  `massive-applications` registers it once per process, before it installs any task context, and
  backs it with the task-local of ADR 0008. The choice of mechanism therefore stays in the layer
  that knows the runtimes. A later switch to thread-locals touches only that layer. The hook adds
  one indirect call to each create, update and drop. That call is small compared to the task-local
  lookup, which the path already pays.
- `scene::submit(&collector, value)` and `submit_to(collector)` go away. Submitting always goes to
  the current task's queue, because every later change of the object follows the hook anyway. The
  `Submit` trait moves from `massive-applications` into `massive-scene`, and the applications
  prelude re-exports it. It is implemented for every object, and for an
  unsubmitted location, which creates its transform before the location that refers to it.

### The task-local owns its change queue

Once handles no longer store a sink, only the owning task touches its change queue. The task pushes
through the hook, and its frame submission drains the queue through the same task-local. So the
task-local owns the queue directly, without an `Arc` and without a `Mutex`. `AnyCollector` is a
plain value whose methods take `&mut self`. The `RefCell` sits in the task-local
(`RefCell<AnyCollector>`), because a task-local hands out only a shared reference, so the reason
for it stays next to its cause.

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
- No code runs while the queue is borrowed. A push converts the change and moves it into the queue,
  and draining moves all changes out with `mem::take`. Neither drops a value, so no `Drop` of a
  handle can run and reach the queue while it is borrowed, and the `RefCell` never fails. A value is
  built from handles before the push borrows the queue. ADR 0008 rejected `RefCell` for borrows that
  cross an `await` or nest, and a push does neither. During unwinding, the hook uses
  `try_borrow_mut`, so a failed borrow there cannot panic a second time.
- `AnyCollector::sink`, `task_context::sink` and the `ChangeSink` trait go away. The hook collects
  each change directly, and nothing sends several scene changes at once any more.
- `task_context::with_changes` goes away. It handed the queue to a closure, which would hold the
  borrow while the closure runs, so any handle used inside it would fail the borrow. Nothing outside
  `task_context` used it.
- The lock between the shell's host task and the render thread (`RenderThreadSubmission` in
  `shell/src/window_renderer.rs`) stays. It hands submissions from one thread to another, so it is a
  real cross-thread queue.

With this, the path of a push no longer contains an atomic operation. It is the hook, the
task-local lookup and a `RefCell` borrow.

### Handles are used inside a task

A handle is created, updated and dropped only while a task context is installed. This is
enforced.

- The hook uses `try_with`. When no task context is installed, a `debug_assert!` fails, and release
  builds log an error. The change is lost, so the object stays in the renderer until the renderer
  ends. `Drop` must not panic, because a panic during unwinding aborts the process.
- An instance's task changes the scene after its final submission, too. Its view parent drops after
  the final submission, and the handles in its movement closures only when its task-locals end.
  So the task's runner drains the queue once more after every task-local has dropped, and the host
  integrates these final changes after the instance's last outputs. The desktop receives them with
  the instance's end (`InstanceEnd` in `desktop/src/instance_manager.rs`). The shell ends with its
  one instance, so it discards them. `InstanceChange::End` goes away, because it only carried the
  release of the view parent. This also covers an instance future that is dropped without ever
  running, because building its runtime failed.
- Tests that submit through an explicit collector install a task context instead. The tests of
  `massive-scene`, which sits below `massive-applications`, register a hook backed by a test queue.

### Call sites

- The clones that only keep a parent alive become ids. A function that only reads a parent
  location, such as the constructors of the project, launcher, title bar and instance presenters,
  `project_matrix_location`, and `View::location`, takes or returns an `Id<Location>`, because the
  parent's owner keeps it alive. mt keeps the handle of its padded content location next to its
  view, and its scroll buckets and lines refer to locations by id. The handles of the scroll
  buckets keep them alive, because the buckets in use always cover every line.
- The launcher presenter moves its transform handle into its movement closure instead of cloning it.
- The instance owns its view parent. `InstanceRoot::new` returns the root together with the handle
  of its view location, whose parent is the content location. `DesktopChange::SpawnInstance`
  carries that handle into `InstanceManager::spawn`, and from there into the `InstanceContext`.
  The presenter keeps the view transform, which places the view location, but no handle of the
  view location itself. `InstanceRoot` loses `Clone`. The shell's host hands its root location to
  its instance the same way.
- The instance background no longer hangs below the view location, which the instance owns. A
  background created on presentation could otherwise take the id of a location that an already
  ended instance had released. It hangs below a second location that the desktop owns and that
  shares the view transform, so it sits where the view sits. ADR 0041 proposes to turn it into a
  background of each view.
- The instance root has a single writer per handle. The presenter writes the parent, the initial
  transform and the initial alpha in `InstancePresenter::new`, before it mounts the movement. Then
  it moves `layout_transform` and `layout_location` into the movement closure.
- `InstancePresenter::present_view` no longer sets the alpha to zero. The alpha is already zero,
  because construction sets it and the movement's `view_alpha` starts at zero. The `modify` that
  starts the fade-in also reaches the same submission, because `Frame::submission` runs the
  movement actions before it drains the queue.
- `InstancePresenter::latest_transform` gives way to `target_transform`, which returns the
  presenter's target, because the presenter can no longer read the transform the closure wrote. The
  two differ only while the instance is still animating. Starting a new instance from it within the
  500 ms of a layout animation lets the new instance appear at the originator's destination instead
  of its position on screen.

### Conditions this design relies on

- **Tasks form a tree.** Changes travel up only through submissions, and siblings do not
  communicate. A handle crosses tasks only from a task to a task it starts, as the view parent
  does. Under this condition, the stage always sees a handle's `Create` before its `Release`. The
  desktop, and the shell's host, push an instance's scene changes into their own queue in the order
  of the instance's output (`apply_instance_change` in `command_dispatch.rs`, `apply_change` in
  `instance_host.rs`). So the instance's earlier changes are queued before the desktop acts on a
  message from it. If a handle ever reaches another task by any other path, its `Release` can
  overtake its `Create`.
- **A scene change batch stays whole.** The stage ends objects at the end of the scene change batch
  that holds their last `Release`, together with what they referred to, so all their deletes land
  in that batch. Each task drains its queue only in its own frame submission, so the changes
  between two submissions of a task land in the same scene change batch, too. The stage must
  neither split a scene change batch nor carry a `Delete` into the next one.
- **An id is valid until the task's next submission.** An `Id` does not keep its object alive. The
  handle it was taken from may drop before the value holding the id is submitted, because the stage
  ends objects only at the end of a scene change batch. The value has to be submitted no later than
  the submission that carries the handle's release. An id kept across a submission can refer to an
  object that has already ended. If the id was reused meanwhile, it refers to another object, and no
  error reports it. Today every value is built shortly before it is submitted.
- **The renderer resolves references only between scene change batches.**
  `SceneState::apply_changes` only writes table rows, and locations are resolved later, in
  `prepare`. So the order of changes inside a scene change batch does not matter, only the state at
  its end. Drop order is not involved.
- **All changes of an object reach one renderer.** This holds today, because the shell draws one
  view into one renderer and the desktop draws into its one window.

## Out-of-process

The out-of-process plan (`docs/out-of-process-applications-plan.md`) moves applications into
their own processes, with scene changes as the boundary. This decision prepares for that in three
ways.

- **Ownership is data.** Each instance gets its view parent from the host. Before this decision
  that was a `Ref`, an `Arc` into the desktop's memory, which no other process can hold. Under this
  decision the instance receives the handle itself, which is an id and a value. Its updates and its
  `Release` are changes that travel over a connection like any other change. A process that owns
  its view parent may change the parent's parent, so the protocol has to check every parent id it
  receives, which it has to do for any location anyway.
- **Handles hold no pointers.** An application process registers its own hook, so its
  `InstanceContext` works without changes to application code.
- **This decision must land before `scene::v1` is frozen.** The plan never edits a protocol version
  after release. `Release` belongs in the scene protocol, and adding it later would need a
  `scene::v2`.

One gap remains. The lifetime stage returns ids to the id generator, and it runs only in front of a
renderer. An application process has no renderer, so the ids of its own objects would never be
released. The plan records this as an open question, together with its candidate answers and the
prior art. To keep every answer possible, the lifetime stage stays self-contained. It turns one
scene change batch into render changes and depends on neither the shell nor the renderer, so an
application process can run it at its own edge.

## Considered Options

- **Counted `Ref`s inside values.** This was the first version of this decision. It broke as soon as
  a `Ref` outlived the handle of the object holding it, as described for ids in values above.
- **A dropped handle hands its value's `Ref`s over to the stage.** `Handle::drop` would suppress the
  releases of the `Ref`s in its value through a thread-local flag, and the stage would release them
  when the object ends. That keeps `Ref` in the values, but a hidden flag would change what
  dropping a `Ref` means, and the stage would need the same record of references anyway.
- **The value lives as long as the object.** That needs shared ownership of the value, which is an
  `Arc` per object again.
- **Arena handles.** The renderer side already is an arena. `Id` is a dense, recycled `u32`, and
  `IdTable` is a vector indexed by it. Arena handles would count nothing, so an object could end
  while a live object still refers to it. Ids are copyable under this decision too, but the stage
  counts the references of every live object.
- **Counts in the renderer.** The renderer would decide lifetimes while it applies changes. A stage
  in front of it stays self-contained, so an application process can run it at its own edge (see
  Out-of-process).
- **Counts in each change queue.** This works, but every queue would carry a count table. The user
  preferred one place that decides lifetimes for all queues.
- **A stored sink in every handle.** Every change of an object would go to the queue it was
  submitted to, so the order of its changes would hold even if siblings communicated, and
  a drop outside a task would be harmless. The cost is one shared `Arc` per handle. Within the tree
  of tasks the order holds without it, so the user chose handles without any `Arc`.
- **`Ref` as a borrow of its `Handle`.** The compiler would prove that a reference never outlives
  its object. But views own both a location and the visuals that refer to it, so they would become
  self-referential.
- **A separate queue for lifetime changes.** It keeps lifetime events out of the change stream. But
  every submission step would need a second path, and the queues would have to be drained together
  to keep a scene change batch whole. The separation is only needed at the renderer, so separate
  types in one queue suffice.
- **The change-queue task-local inside `massive-scene`.** It avoids the hook's indirect call. But
  `massive-scene` would decide how a task is identified, and it would need tokio's `rt` feature.
- **A read path on `Movement` for `latest_transform`.** It would keep today's exact behavior, at the
  cost of a new read API in `massive-animation` for one caller.
- **Counted references (`Ref<T>`).** An earlier version of this decision kept them, with a
  `Retain` change for each one. Client code held them to keep an object alive without writing it.
  Every such use was either a temporary that only needed the id, or a keep-alive that a handle or
  a referring object already provides. The one reference that crossed tasks, an instance's view
  parent, became a handle the instance owns. Removing them leaves one way to keep an object alive
  and one change fewer in the protocol.
- **A check for stale ids.** In debug builds, `Id` could carry a generation that the stage compares
  with the live object's, so a reused id would fail loudly. Alternatively, an id passed between
  functions could be a token that panics when it is dropped without being placed in a value. The
  token checks the wrong thing, because dropping an id is harmless while dropping its source too
  early is not. The user kept the documented rule without a check.
- **Builders that borrow the sources of their ids.** A value under construction would borrow the
  handle each id was taken from, so the compiler would prove that the sources live until the value
  is submitted. It needed a lifetime on every builder, on `update` and on `set_parent`, which was
  too much for a gap that ending objects at the end of a scene change batch already closes for every
  value submitted before the task's next submission.

## Consequences

- A handle that is forgotten with `mem::forget` leaks its object, and so do locations whose parents
  form a cycle. Neither leaves a reference dangling.
- Building a value sends nothing, because it holds ids, so values can be built and cloned freely.
  The desktop's hover and zoom level indicator still set only their shapes through `set_shapes`,
  which leaves their location and decal order alone.
- A handle holds its value inline, so a type with many handles grows. `InstanceRoot` holds six, so
  the two enum variants that carry one, `DesktopCommand::StartInstance` and
  `DesktopChange::PresentInstance`, box their payload. Unboxed, every command and every change
  would be as large as these variants.
- Switching the hook to thread-locals later requires that every task holding handles runs on a
  current-thread runtime. Today the shell's runtime and instances with `RuntimeKind::MultiThread`
  move tasks between threads.
- Creating an object still takes a process-wide lock, the one `Mutex` of the id generator, which
  every task that creates objects and every lifetime stage share. A follow-up could give each task
  its own range of ids, if that lock ever shows up in a profile.
