# Bounded graceful application shutdown

A native window close is a lifecycle request for the whole application, not a focus-targeted input event. The shell forwards the close request through the application event path to every live instance, regardless of focus; the desktop coordinates graceful shutdown by requesting all instances to end, processing their final submissions, and awaiting their completion before releasing the renderer and native window. One application-wide monotonic shutdown deadline bounds this drain; when it expires, the desktop returns an error and the shell closes through its normal application-ended path, with no final-submission guarantee after the deadline.

## Status: accepted

## Considered options

- **Route close only to the focused view or originating instance.** Rejected: closing the native application window is not a focus operation, and unfocused instances must receive the same lifecycle signal.
- **Handle close as a desktop-only stop command.** Rejected: it bypasses the normal instance event path and can tear down the desktop while an instance is still dropping its context and submitting final changes.
- **Exit the process immediately for every close request.** Rejected as the normal path: it avoids races but discards orderly instance termination and final submissions. A deadline error remains the timeout fallback, allowing the shell's normal application-ended path to close the event loop.
- **Use independent per-instance timeouts or a forced-shutdown state.** Rejected: one application-wide deadline gives a hard upper bound, and after expiry the desktop ends rather than modeling another lifecycle state.

## Consequences

- `CloseRequested` remains a `ViewEvent`, but it is a broadcast lifecycle event rather than a focus-routed input event.
- Instances must stop normal frame production after receiving the close request and return through their ordinary completion path.
- The submission receiver remains available until all graceful instance completions and final submissions have been handled.
- Instance failures are recorded while remaining instances continue draining; a user-requested close still completes unless the shutdown deadline expires.
- The deadline fallback may abandon pending rendering, scene changes, and final submissions, but it must end the application with an error and log unfinished instance diagnostics.
