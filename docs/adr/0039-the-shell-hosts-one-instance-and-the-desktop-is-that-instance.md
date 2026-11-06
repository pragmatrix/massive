# The shell hosts one instance, and the desktop is that instance

## Status: accepted

## Context

There were two kinds of program. A shell application took an `ApplicationContext`, opened its own
windows and rendered into them. The examples and the desktop were of this kind. A desktop
application was an `Application` whose instances took an `InstanceContext`, created views and
submitted instance changes, which the desktop drew. mt's terminal was of this kind. An example could
not run in the desktop, and an application could not run on its own. The out-of-process plan also
expects that an application which ships in-process runs standalone in the shell, with its own window.

## Decision

Every program is an `Application`, and every running program is an instance with an
`InstanceContext`. `ApplicationContext` is gone.

- The desktop runs many instances. The shell runs exactly one, through `shell::run(application,
  font_policy)`. Both hosts take the same `Application` value, which now lives in
  `massive-applications`.
- An application declares its draw target, where its instances draw their content. With
  `DrawTarget::Views`, the default, the instance creates views and its host draws them. The shell
  draws the instance's one view into a window of the view's size, so an example runs on its own as
  well as in the desktop. With `DrawTarget::Windows`, the instance opens its own windows through
  `InstanceContext::new_window` (the `InstanceWindows` trait of `massive-shell`) and renders into
  them. Only the shell runs such an instance, and the desktop's registry rejects one.
- The desktop is the shell's instance with `DrawTarget::Windows`. `DesktopEnvironment::run` wraps it
  into an `Application` and calls `shell::run`, so mt never sees the shell.
- The shell runs its instance as a task on its own tokio runtime, or on the caller's, which must be
  a multi-thread runtime. ADR 0009 gives every desktop instance a dedicated thread, so that a long
  instance poll cannot starve the desktop task on a shared pool. Under the shell, the only work
  beside the instance is the host task of an instance that draws into views, and the multi-thread
  worker pool keeps it running. So the shell's instance needs no thread of its own, and the
  application's runtime kind applies only in the desktop. The desktop keeps running on the shell's
  user-interactive worker pool, as it did before.
- `InstanceContext::wait_for_events` is the one way an instance receives events. It receives a
  batch and starts an animation cycle at each tick's vblank time, advancing the movements, so
  movements work under both hosts. Movement completion events arrive as `InstanceEvent::Custom`.

## Considered Options

- **A separate window handle next to `InstanceContext`** (`shell::run_host(fn(InstanceContext,
  Windows))`). This keeps the window capability in the type system, but needs two shell entry points.
  The user chose to integrate window creation into `InstanceContext`.
- **The desktop renders through a view, like every instance.** This is the most uniform option, but
  the view vocabulary lacks native fullscreen, background color and the camera, and the desktop
  would lose its direct, resize-synchronous renderer.
- **The desktop collects instance changes and converts them back to scene changes.** This would
  avoid the declared draw target, but every desktop frame would pass through a conversion, and the
  desktop's own `submit` calls would need the instance change type. The kind of a task's change
  queue is fixed when the task starts (ADR 0008), so the shell must know before the start whether
  the instance draws into its own windows. That is why the application declares it.

## Consequences

- `InstanceContext::view` and `submit` panic for an instance that draws into its own windows, and
  `new_window` fails for one that draws into views. The type does not separate the two.
- The shell's host supports one view per instance. A second view ends the shell with an error, and
  `ViewChange::Resize` is ignored, because the shell does not resize its windows.
- Under the shell, an instance has no desktop socket, so `process_environment` is empty and the
  shell drops configuration requests. No tool calls arrive. The processes the instance starts still
  inherit the shell's own environment, so a shell started from a desktop terminal passes that
  terminal's `MASSIVE_SOCKET` and `MASSIVE_INSTANCE` on.
- A close request of the shell's window reaches an instance that draws into views as the shutdown
  request, as the desktop sends it.
- For an instance that draws into views, the shell's host task is the only party that reports the
  instance's end. It closes the view's window first, so the event loop handles the window's
  destruction before it ends. An instance that draws into its own windows closes them before its
  task reports the end.
- An application's run function is an `Fn`, because the desktop starts many instances of one
  application. An application that only the shell runs and that hands state to its one instance,
  such as the desktop or the logs example, takes that state out of a `Mutex<Option<_>>` on the one
  call and expects it to be there.
- When the shell's host fails, for example on a second view, it asks the instance to shut down and
  ends with its own error, without waiting for the instance.
- An instance that draws into views and the shell's host task share the shell's runtime, whose
  worker pool keeps the host responsive while the instance works.
- The shell does not catch an instance's panic. mt and massive build with `panic = "abort"`, so a
  panicking instance ends the process. The desktop catches panics only so that its tests, where
  cargo forces unwinding, see them as the instance's result (ADR 0009).
