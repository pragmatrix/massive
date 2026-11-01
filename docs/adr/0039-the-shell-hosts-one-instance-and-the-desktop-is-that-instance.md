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
- An application declares who renders its instances' content. With `DrawTarget::Views`, the default,
  the instance creates views and its host draws them. The shell draws the instance's one view into a
  window of the view's size, so an example runs on its own as well as in the desktop. With
  `DrawTarget::Windows`, the instance opens its own windows through
  `InstanceContext::new_window` (the `InstanceWindows` trait of `massive-shell`) and renders into
  them. Only the shell runs such an instance, and the desktop's registry rejects one.
- The desktop is the shell's instance with `DrawTarget::Windows`. `DesktopEnvironment::run` wraps it
  into an `Application` and calls `shell::run`, so mt never sees the shell.
- The shell runs its instance like the desktop runs one (ADR 0009): on a dedicated thread, under the
  runtime its application names, with the panic handling both hosts share. The shell's thread runs
  at user-interactive priority, which the desktop had on the shell's worker pool before.
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
  avoid the declared rendering, but every desktop frame would pass through a conversion, and the
  desktop's own `submit` calls would need the instance change type. The kind of a task's change
  queue is fixed when the task starts (ADR 0008), so the shell must know before the start whether
  the instance renders itself. That is why the application declares it.

## Consequences

- `InstanceContext::view` and `submit` panic for an instance that renders itself, and `new_window`
  fails for one whose host renders. The type does not separate the two.
- The shell's host supports one view per instance. A second view ends the shell with an error, and
  `ViewChange::Resize` is ignored, because the shell does not resize its windows.
- Under the shell, an instance has no desktop socket, so `process_environment` is empty and the
  shell drops configuration requests. No tool calls arrive.
- A close request of the shell's window reaches a host-rendered instance as the shutdown request,
  as the desktop sends it.
