# Duplicate configuration names are allowed; nodes are identified by tag

## Status: accepted

> The tag mechanism is superseded by
> [ADR 0013](./0013-desktop-configuration-is-json-derived-from-the-aggregate.md):
> the document is derived from the aggregate, so changes address ids directly
> and no per-node identity lives in the file. The duplicate-name and
> nearest-match decisions below still hold.

## Problem

The desktop configuration's KDL document (ADR 0006) is keyed by name, and every
change resolved its target by looking the name up in the live aggregate:
`ConfigurationDocument::apply` took the `&DesktopConfiguration` beside the change
and read the project/launcher name out of it, then found the node by
`(node kind, first string argument)`. That coupled the document view to the live
model in both directions:

- The document was written before the live model, because a removal must still
  resolve its name while the aggregate held the entity.
- Names had to be unique document-wide (projects) and among siblings (launchers),
  enforced by appending ` 2`, ` 3`, ... when a change introduced a collision.
- A hand-edited duplicate silently broke: the name lookup found the *first* node,
  so removing the second `project "work"` removed the first. Parse did not reject
  duplicates, so the file and the live model diverged with no error.

`reject_emptying_removal` had a second, related defect: a cascading project
removal emits its `RemoveLauncher` changes before the `RemoveProject` change, so
the document rejected the transaction only after the launcher changes had already
been applied to both the document and the live model — a half-applied, then
rejected, transaction.

## Decision

The document identifies its nodes itself, by a **tag assigned per parsed or added
node**, and duplicates are allowed.

- The tag rides in the node's `span`, a field the parser fills for diagnostics and
  stringification never writes. `KdlNode::span` is excluded from `PartialEq`/`Hash`,
  so document comparison is unaffected, and the file stays byte-identical. This is
  a deliberate misuse of a source-mapping field: it is the only per-node storage
  kdl offers that is neither serialized nor compared. A comment at the map marks it.
- `NodeTags` (in `persistence/document.rs`) holds `ProjectId`/`LaunchProfileId` →
  tag plus a counter, and is filled during the parse so each node is tagged with
  the id it was parsed into — duplicates included. `apply_change` addresses nodes
  through the tags only, so `ConfigurationDocument::apply(change)` no longer takes
  the aggregate and the document is a self-contained view again.
- Names are no longer deduplicated. A user-chosen name is stored as it is; only the
  generated default names (`New Project`, `New Launcher`) get an index, allocated
  as the lowest index not already taken and reused once freed.
- Name-based lookups resolve to the **nearest** entity, because a name is what the
  user has: `RemoveLauncher { name }` picks the same-named launcher in the current
  project with the smallest matrix distance to the focused launcher, and
  `RemoveProject { name }` picks the same-named project nearest in document order to
  the focused launcher's project. Ties and a missing focus fall back to the first
  match, matching how `startup` resolves a name.
- The "at least one launcher" invariant moves to the planner (`plan_project`),
  where the whole command is visible and a rejection emits no change at all. This
  closes the half-applied transaction and removes the document's last piece of
  policy. Superseded by
  [ADR 0012](./0012-a-configuration-without-a-launcher-is-given-one-at-load.md):
  the invariant is dissolved — the parse gives a configuration without a launcher
  one — so the planner no longer rejects on its account.

## Considered options

- **Keep name addressing and reject duplicates at parse.** Rejected: it turns a
  hand-editable file into one where a copy-pasted block fails the whole session,
  and it keeps the two representations coupled.
- **Store an index into the node vectors.** Rejected: every structural edit must
  maintain the indexes, including the `startup` node inserted at position 0 which
  shifts all project indexes.
- **Write an `id=` property into the document and strip it on serialize.** Rejected:
  the held document then differs from the written text, and every serialization
  needs a filtered copy.
- **A raw pointer to the node.** Rejected: `nodes_mut()` hands out `&mut Vec`, so
  any insertion reallocates and dangles the pointer.
- **Abstain from name-based lookup entirely (ids only).** Rejected: the launcher
  presenter's menu addresses projects and launchers by name over the IPC boundary
  (`ConfigurationRequest`), so the names must still resolve to something.
