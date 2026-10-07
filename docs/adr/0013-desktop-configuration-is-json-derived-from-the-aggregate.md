# The desktop configuration is a JSON document derived from the aggregate

## Status: accepted

Supersedes [ADR 0006](./0006-desktop-configuration-is-name-keyed-kdl.md) (the
name-keyed KDL file edited surgically) and the tag mechanism of
[ADR 0010](./0010-desktop-configuration-names-may-repeat.md).

## Problem

The desktop configuration was persisted as a name-keyed KDL file
([ADR 0006](./0006-desktop-configuration-is-name-keyed-kdl.md)), edited
surgically so hand-written comments and formatting survived every automated
write. The surgical-edit design carried its own machinery: node identity tags
riding in the parser's `span` field (ADR 0010), a KDL↔JSON conversion for spawn
parameters, comment-aware formatting bookkeeping on every insert and remove,
and a document view that had to be kept in sync with the live aggregate by
mirroring every change twice.

The complexity bought comment preservation — and nothing else. No one edits the
file by hand in practice, and every format-preserving edit path was a place the
document and the live model could diverge.

## Decision

The configuration is stored as JSON in `~/.massive/desktop.json`, and the file
is **derived from the aggregate** at every flush rather than edited alongside
it.

- The document is the fractal view of the configuration (ADR 0011): the top
  level is `startup` plus the root project's `slots`; a slot is `at` (a
  `[column, row]` pair) plus exactly one of `launcher` or `project`; a project
  node is `name` plus `slots`, recursing. The document root is the one node
  without a name — the terminal synthesizes the root project `Projects`.
- Ids are not serialized. Projects and launchers get fresh ids at load; the
  document addresses content by nesting and placement alone, and the `startup`
  address path resolves by name as before.
- A change is applied to the aggregate and marks the document dirty; the flush
  at the end of a transaction serializes the aggregate into the file. There is
  no separate edit path, no tags, and no KDL node bookkeeping — the document
  cannot diverge from the model it is derived from.
- The file is written atomically (temp file plus rename) and pretty-printed.
  Comments and formatting are not preserved, because the file carries none:
  every write is a fresh serialization.
- The `kdl` dependency leaves the desktop crate. The terminal's own settings
  (the `mt` crate) keep their KDL configuration; this ADR covers only the
  desktop configuration document.

## Consequences

- `ConfigurationDocument` shrinks to a path plus a dirty flag; `apply` cannot
  fail, because there is no document state an edit could fail to resolve.
- `SetStartupPath` must be retained by the aggregate (it previously lived only
  in the document), so the flush can serialize it. The aggregate's
  `startup_path` is the persistence source; the resolved launcher id stays
  fresh per session.
- Duplicate names stay legal (ADR 0010's decision), but the tag mechanism that
  made them addressable is gone: with the document derived from the aggregate,
  a change addresses the aggregate's ids directly and the serialization walks
  the same ids, so duplicates need no per-node identity in the file.
- Hand-editing the file is a plain JSON edit; a syntax error fails the load
  with the file path in the message, and the default configuration is written
  only when the file is missing, not when it is invalid.

## Considered options

- **Keep KDL with the surgical edits.** Rejected: the maintenance cost of the
  tag mechanism, the parameter conversion, and the formatting bookkeeping
  outweighs comment preservation for a file that is effectively
  machine-managed.
- **Serialize the aggregate directly with serde derives.** Rejected: the
  aggregate's shape (flat project list, id links, the `Slot` redundancy between
  `content` and `launcher`) is not the file's shape; a separate document model
  keeps the fractal file format independent of the runtime representation.
- **Migrate existing KDL files automatically.** Rejected: no other users exist;
  the current configuration is converted once, by hand.
