# A configuration without a launcher is given one at load

## Status: accepted

Supersedes the "Boot still requires a launcher" section of
[ADR 0011](./0011-desktop-is-a-fractal-of-projects-in-slots.md), and the
"at least one launcher" invariant of
[ADR 0010](./0010-desktop-configuration-names-may-repeat.md).

## Problem

ADR 0011 rejected a launcher-less configuration at load and ADR 0010 moved the
"at least one launcher" invariant into the planner, so that a rejection is seen
when the whole command is visible and emits no change. The planner check was
`launcher_count() > launcher_count_of_subtree(project)`, reached only from the
removal of a project.

That placement has two defects:

- It inspects the pre-state, so it rejects an operation whose end state is fine:
  replacing the configuration's only launcher-bearing slot is a clear plus an
  assign, and the clear is planned before the assign, so the count momentarily
  reads zero and the replace is refused.
- `RemoveLauncher` of the last launcher is not checked at all: the identical
  invariant is not enforced on the launcher path, so the same end state is
  reachable there without a rejection.

The invariant is also not a property of any one command. It is a precondition of
*booting*, and policing it per command — in a form that can only see the
pre-state — puts a boot requirement in the middle of the command vocabulary.

## Decision

A configuration that defines no launcher is legal. Boot's requirement — that a
launcher exists — is satisfied at load instead of enforced by a rejection.

- Parsing stays a pure read of the document; giving a launcher-less configuration
  a launcher is a separate step that runs after it, and it adds the launcher to
  the root project: mode `visor`, no spawn parameters, and the first free
  placement of the root matrix. The launcher enters the document as a real
  `launcher` node, so the first configuration change that rewrites the file
  persists it, and a re-parse finds it rather than adding a second one. A session
  that changes nothing leaves the file byte-identical and re-synthesizes on the
  next boot.
- The planner's launcher-count guard is deleted, and with it the
  `RemoveProject`-specific rejection. Removing the last launcher during a session
  is legal: every instance shuts down with it, and because configuration requests
  ride an instance there is no request channel left until a restart, which gives
  it a launcher again.

## Consequences

- `boot_launcher()` is `Some` for every loaded configuration, because the load
  gives a launcher-less one a launcher; the boot flow's `expect` holds without a
  load-time gate.
- The empty configuration is reachable at runtime and recoverable only by a
  restart (or by hand-editing the file). This is accepted: the alternative was a
  pre-state rejection that refused valid replacements.
- Giving a launcher-less configuration a launcher happens in one place; no command
  reasons about "no launcher" at all, so the clear-then-replace path is no longer
  guarded on that account.

## Considered options

- **Keep the invariant in the planner, made refill-aware.** It would have to
  receive what re-fills the cleared slot to check the end state, and it would
  still only cover the project path. Rejected: more machinery for a
  boot precondition, and the launcher path stays unguarded.
- **Synthesize at load but do not persist it.** Rejected: the file and the live
  model would disagree about whether a launcher exists; the next boot would
  create a different launcher, so any adjustment the user makes to the first one
  could not be expressed in the document.
- **Reject a launcher-less file at load.** Rejected: it is the ADR 0011
  behaviour, and it turns a hand-editable file that lost its last launcher into
  one that fails the whole session.
