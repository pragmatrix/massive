# A watcher applies edits of the desktop configuration file at once

## Status: proposed

Amends [ADR 0037](archive/0037-desktop-configuration-is-kdl-reconciled-from-the-aggregate.md) where it
says that nothing is kept between persists. Replaces the mechanism of
[the file observer plan](../desktop-configuration-file-observer-plan.md), which builds on the
surgical edits of [ADR 0006](archive/0006-desktop-configuration-is-name-keyed-kdl.md) and on types
that no longer exist.

## Context

The desktop configuration is stored in `~/.massive/desktop.kdl`. Every persist derives a fresh
document from the aggregate and reconciles it into the file on disk, so the file keeps its comments
and formatting ([ADR 0037](archive/0037-desktop-configuration-is-kdl-reconciled-from-the-aggregate.md)).
The persist reads the file again each time, so comments added while the terminal runs survive.

The aggregate in memory wins at every persist, because the written file reads as exactly the
aggregate. A structural edit made by hand while the terminal runs is therefore undone by the next
change made in the terminal:

| Hand edit on disk since the last persist | At the next persist |
|---|---|
| A launcher or project added | Removed, together with the comments above it |
| A slot removed | Written again, without its former comments |
| A slot moved to another `at` | Moved back, keeping its comments |
| A value changed | Replaced by the value in memory |

The first row loses work without notice.

## Decision

- **A watcher applies every edit of the file at once.** When the file changes on disk and loads, the
  desktop turns the difference between the aggregate and the loaded configuration into
  configuration changes and applies them as one transaction. The aggregate then follows the file,
  so at the next persist the file differs from the aggregate only by the terminal's own change, and
  the reconcile pairs nearly every node by its position.
- **The persistence keeps the document in memory.** `ConfigurationPersistence` holds the
  `KdlDocument` it last loaded, reloaded, or wrote, with its comments. A persist reconciles the
  fresh document into that document, writes the result, and keeps it. It does not read the file.
  A reload replaces the document in memory with the one parsed from the file. Without the watcher,
  keeping the document in memory would overwrite every hand edit at the next persist, comments
  included. With the watcher, the document in memory always holds the last edit that loaded.
- **The persistence remembers the hash of the text it last loaded or wrote.** A persist that finds
  a different hash on disk has met an edit the watcher has not applied yet. It applies the reload
  first and persists after it, so neither side overwrites the other.
- **A file that does not load leaves the session as it is.** The watcher logs an error with the
  location, as the load does, and tries again at the next change of the file. A half-finished edit
  therefore neither breaks the session nor gets overwritten.
- **The reconcile and the format of ADR 0037 do not change.** The terminal's own changes still have
  to be merged into a file that holds comments.

## Open questions

- **Running instances of a removed launcher.** The desktop policy keeps slot content while
  instances run below it (`removing_slot_content` in `desktop/src/policy/when.rs`), so stopping
  work is always a deliberate step. An edit that removes such a launcher must either be refused as
  a whole, keep the launcher until its instances end, or stop them. The file observer plan chose to
  stop them, before that policy existed.
- **Polling or file system events.** The file observer plan chose polling with a content hash,
  which needs no new dependency and handles the atomic rename that editors and the persist use.
- **Duplicate names.** The file observer plan made duplicate names a reload error. Duplicate names
  are legal today ([ADR 0010](archive/0010-desktop-configuration-names-may-repeat.md)), and the
  reconcile pairs nodes by position first.
- **How a moved slot is told apart from a removal and an addition.** The difference between two
  configurations can pair slots by position and name, like the reconcile does. A renamed slot that
  also moved then appears as a removal and an addition.

## Consequences

- Hand edits take effect while the terminal runs, and no structural edit is undone by the next
  persist.
- The persist no longer reads the file, so it no longer needs its branch for a file that does not
  load. The watcher's error replaces it.
- The persistence keeps state between persists: the document and the hash. ADR 0037 avoided that
  state, because a persist that works from an old document overwrites hand edits. The watcher keeps
  the state current, so the reason no longer holds.
