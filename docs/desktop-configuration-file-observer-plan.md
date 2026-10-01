# Desktop configuration file observer plan

Status: **draft, not approved**. Decisions were taken in a session discussion
(2026-09-29); one follow-up decision is still open (see "Open decisions").

## Goal

Make the desktop configuration file live again during a session: an observer
watches `~/.massive/desktop.kdl`, reloads it when it changes on disk, and drives
the running session to the new configuration by computing a delta between the
current live model and the reloaded one. External edits (by hand or another
tool) take effect without restarting; interactive edits from the UI keep working
unchanged.

The file wins. A reloaded document replaces the in-memory document wholesale —
its bytes, comments, and formatting are kept exactly — and only the *live model*
is driven by deltas. The delta must never round-trip through the file-editing
machinery (`apply_change` on `KdlDocument`), because the freshly parsed document
already *is* the target file state.

## Current situation

The configuration is a name-keyed KDL file (ADR 0006: launchers get fresh UUIDs
each session, so the file cannot reference ids). At boot, `load_configuration`
(`massive/desktop/src/desktop.rs`) writes the built-in default when no file
exists, then `ConfigurationDocument::load`
(`massive/desktop/src/projects/persistence/configuration.rs`) parses the
document, derives the live `DesktopConfiguration` aggregate, and registers the
document's names in `ConfigKeys` for later id → name resolution.

After boot, change flows one way:

1. User commands are planned into `ProjectChange`s
   (`massive/desktop/src/desktop_system/change.rs`) and applied by
   `apply_project_change`
   (`massive/desktop/src/desktop_system/command_dispatch.rs`), which updates the
   live aggregate and the presenters.
2. The same change is mirrored into the parsed `KdlDocument` via
   `ConfigurationDocument::apply` → `ConfigKeys::map_change` (name dedupe,
   registration bookkeeping) → `apply_change`
   (`massive/desktop/src/projects/persistence/document.rs`), a surgical node
   edit that preserves user comments and formatting.
3. `flush()` writes the document once per transaction, atomically.

The change stream is the only thing keeping document and live model aligned,
which is exactly what an external edit breaks: today the session neither sees
the edit nor recovers from the resulting divergence.

The live aggregate keeps each project's launchers sorted by placement and
answers assignment and shift queries from placements alone
(`massive/desktop/src/projects/configuration.rs`). The delta application must
keep that invariant, which is why additions are ordered after removals.

## Decisions (taken)

- **Watch mechanism: polling + content hash.** No `notify` dependency; ~500 ms
  poll; hash the file text and act only when it differs from both the
  last-loaded and last-written hash. The hash naturally handles atomic
  rename-saves (which our own `flush()` and most editors use) and kills
  self-echo, so there are no grace-period hacks.
- **Invalid file: keep the last good state, log a warning.** Parse and invariant
  failures are expected mid-edit; retry on the next poll when the file settles.
  The session never dies from a bad hand-edit.
- **Duplicate names: a parse error.** Duplicate project or launcher names make
  the reload fail (keep last good state, warn, retry), even though
  `configuration_from_document` currently tolerates duplicates and warns only
  for `startup` resolution. See "Open decisions" for where the check lives.
- **Running instances of removed launchers: shut them down.** An external edit
  that removes (or rewrites) a launcher stops its running instances, like a
  user-initiated `StopInstance`. No orphaned, unlinkable instances.
- **Changed `startup` profile: next session only.** The live model records the
  new startup id; nothing reboots mid-session. Matches today's semantics, where
  `apply_project_change` ignores `SetStartupProfile` (consumed at boot).

## Proposed direction

### 1. Observer (`persistence/observer.rs`)

New module. Polls the file, computes a hash, and only *signals* — it emits a
new `DesktopEvent::ConfigurationFileChanged` into the desktop event loop
(`massive/desktop/src/desktop.rs`). All state mutation stays on the loop's
thread, serialized with user transactions; the observer itself holds no model
state.

The observer must learn the hash of the last written text so a `flush()` never
looks like an external edit. The cleanest seam: when the delta application
replaces the in-memory document (step 4), the new hash is recorded as both
"last loaded" and "last written".

### 2. Target construction

On signal: read text → hash → parse → `configuration_from_document` → target
`DesktopConfiguration` with fresh ids. Parse/invariant failure: keep last good
state, `log::warn`, retry on the next poll.

### 3. Delta computation

A new pure function, the real work of this change:

```text
diff(current: &DesktopConfiguration, target: &DesktopConfiguration)
    -> Result<Vec<ProjectChange>>
```

Matching is by name (per project for launchers):

- Launchers present in both: placement-only change → `MoveLauncher`; anything
  else changed (mode, params, name is the match key so a rename is remove+add) →
  `RemoveLauncher` + `AddLauncher` with a fresh id. There is no "update params"
  variant in the change vocabulary, so replace is forced.
- New launchers → `AddLauncher` (fresh id); gone → `RemoveLauncher`.
- Projects present in both (name match): keep. New → `AddProject`; gone →
  `RemoveProject` only — its launchers disappear with it, and the delta must
  *not* also emit their removals or the double-removal hits presenter errors.
- `startup`: resolved-name changed → `SetStartupProfile(Some(target id))` /
  `(None)` when cleared.

Ordering: all removals first (frees slots), then additions in document order,
then startup. Placement collisions are impossible by construction because
removals precede additions. A delta that would remove the last launcher is
rejected as a whole (the "at least one launcher" invariant
`reject_emptying_removal` enforces for interactive edits).

Because `diff` is pure and name-keyed, its table-driven tests need no file
system.

### 4. Applying the delta

A synthesized transaction, not bare `ProjectChange`s:

1. `StopInstance`-style changes for every running instance whose launcher the
   delta removes — *before* the launcher removal, so presenter cleanup sees a
   consistent state.
2. The `ProjectChange` deltas through the existing `apply_project_change`
   (presenters and scene update for free).
3. **Skip `persist_project_change`.** The file is already the truth; writing
   back would be a no-op at best and self-echo at worst. Either a mode on the
   dispatch or a separate entry point.

Then one new method on `ConfigurationDocument`:

```text
replace(document, configuration)
```

Swaps in the freshly parsed `KdlDocument`, rebuilds `ConfigKeys` from the
**post-delta live model** (`registered_from(&live)` — the live ids are the ones
future interactive edits must resolve against; the document's own ids from the
reload are throwaways), and clears `changed`. No write occurs; disk and memory
agree byte-for-byte. `flush()`'s no-op path (`changed == false`) is untouched.

### 5. Boot-flow interaction

The boot flow (`load_configuration` → `DesktopSystem::new` → `Setup` transaction
re-applies commands derived from the aggregate) is untouched. The observer
starts after boot; the first poll hash seeds from the text already loaded, so a
file that changed between load and observer start is caught immediately.

## Implementation steps, in order

1. Decide the open duplicate-validation question (below); if shared, extend
   `configuration_from_document` first.
2. Add the `DesktopEvent::ConfigurationFileChanged` variant and wire the
   observer signal into the event loop.
3. Add `ConfigurationDocument::replace`.
4. Add the skip-persistence dispatch mode / entry point.
5. Implement `diff` with its table-driven tests (add, remove, move,
   params-change, rename → remove+add, emptying-removal rejected, duplicate
   names rejected upstream).
6. Implement the observer loop (poll + hash) with a temp-file integration test,
   following the existing `configuration.rs` test style.
7. Implement the transaction: instance shutdown sweep + delta application +
   `replace`.

## Open decisions

- **Where the duplicate-name check lives.** Sharing it in
  `configuration_from_document` keeps file and session rules identical, but a
  duplicate-named config that boots fine today would start being rejected at
  boot too. Scoping it to the reload path keeps boot lenient but leaves two
  rule sets. Default when implementing: shared check in
  `configuration_from_document` (one rule), flagged here until confirmed.

## Testing

- `diff`: table-driven, pure, no file system (see step 5's list).
- Observer: integration test with a temp file — write valid text, expect
  signal; write invalid text, expect no signal and a retry that succeeds once
  valid again.
- `ConfigurationDocument::replace`: after replace, an interactive
  `AddLauncher` for a post-delta id resolves names without "never registered"
  errors, and `flush()` is a no-op until the next change.
- End-to-end (manual or integration): external edit while instances run —
  removed launchers stop their instances; additions appear in the matrix;
  comments and formatting in the file survive untouched.
