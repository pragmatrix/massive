# Desktop configuration as KDL: implementation plan

Status: **approved, not started**. Written from a session discussion on 2026-11-09. It implements
[ADR 0037](adr/0037-desktop-configuration-is-kdl-reconciled-from-the-aggregate.md), which holds the
format and the decisions behind it. This plan holds only the steps.

## Starting point

The desktop configuration is stored as JSON in `~/.massive/desktop.json`
([ADR 0013](adr/0013-desktop-configuration-is-json-derived-from-the-aggregate.md)). The code lives
in `desktop/src/projects/persistence/`:

- `persisted.rs` defines `PersistedConfiguration`, a typed tree of the document with serde
  derives. It converts between the tree and the aggregate (`RuntimeConfiguration`) in
  `into_configuration` and `from_configuration`. It also holds the built-in default configuration.
- `configuration.rs` holds `ConfigurationPersistence::persist`, which serializes the tree and
  writes it, and `parse_configuration`, which parses the file and checks the nesting limits.
- `mod.rs` holds the file name and the atomic write.

## Shape of the change

The typed tree stays as the layer between the aggregate and the file, so the conversions to and
from the aggregate do not change. Only the layer between the tree and the text is replaced:

```
RuntimeConfiguration  <->  PersistedConfiguration  <->  KdlDocument  <->  desktop.kdl
     (unchanged)              (serde removed)          kdl_format.rs     reconcile.rs on write
```

- `kdl_format.rs` converts between a `KdlDocument` and the typed tree, in both directions. It owns
  every format rule of ADR 0037: the `at` string, the omitted defaults, the `mcp` node, the shapes
  of param nodes, and the strict rejection of unknown nodes and properties.
- `reconcile.rs` holds one pure function. It takes the document parsed from the file and the fresh
  document built from the aggregate, and it returns the document to write.

## Steps

All steps happen on one branch of massive, which becomes one PR. mt then moves its submodule
pointer. Each step is one commit and leaves the tests passing.

### 1. KDL format, written fresh

- Add `kdl` to `desktop/Cargo.toml` from the workspace. Both workspaces already pin 6.7.1.
- Write `kdl_format.rs`. Parse with `KdlDocument::parse`, the same KDL v2 parser that mt's
  `terminal.kdl` uses. Writing builds a document and autoformats it.
- Remove the serde derives from the typed tree in `persisted.rs`. Rewrite the default
  configuration in KDL.
- Add the rule that refuses an array containing an object or another array to
  `desktop/src/policy/when.rs`, next to `nesting_params` (ADR 0029). Apply it in
  `desktop/src/tools/desktop_tools/schema.rs`, where the MCP tools check params, and in
  `parse_configuration`.
- Rename `CONFIG_FILE_NAME` to `desktop.kdl`. Each persist writes the fresh document. This step
  does not keep comments yet.
- Convert the JSON test fixtures to KDL. About 41 fixtures in 9 files use them, most of them in
  `desktop/src/desktop_system/system_tests/` and in `persisted.rs`.

### 2. Reconcile

- Write `reconcile.rs` as ADR 0037 describes. The function works on the children of one node and
  calls itself for every matched pair, so the same rules apply to slots, spawn parameters, and
  nested param objects.
  - The first pass pairs nodes by node name plus the `at` property. A node without `at`, such as
    `startup`, `mcp`, or a spawn parameter, pairs by its node name alone.
  - The second pass pairs the nodes left over on both sides by node name plus first argument, so
    a moved slot keeps its comments and its subtree.
  - A matched node keeps its comments and formatting. When its values differ, all of its values
    are replaced by the fresh ones.
  - An unmatched node in the existing document is removed together with the comments above it.
    An unmatched fresh node is inserted at its fresh position.
- `persist` reads and parses the file, reconciles, and writes the result atomically. A file that
  does not parse is not overwritten. The persist logs a warning and writes nothing.

### 3. Documentation

- Set ADR 0037's status to accepted and implemented. Point ADR 0013's note at the implementation.
- Replace `desktop.json` with `desktop.kdl` in `docs/security.md`.
- Update the module comments of the persistence module.
- In mt, update `.agents/skills/run-mt/SKILL.md`, which tells how to seed a scratch `desktop.json`.

## Tests

Each test pins a rule that could otherwise break without notice:

- A comment above a matched node survives a persist.
- A moved slot keeps its comment and, for a project, its whole subtree.
- A removed node takes the comments above it along.
- A node whose value changed carries the fresh value.
- The reconciled document parses to the same typed tree as the fresh document.
- The param shapes: one argument, several arguments, no arguments, a children block, and an
  invalid mix.
- An array containing an object is refused by the load and by the MCP tool.
- An unknown node or property fails the load and names the node.
- A file that does not parse is left untouched by a persist.

The existing tests of the nesting limits and of the round trip through the aggregate keep their
meaning. Only their fixtures change to KDL.

## Not in scope

- Reloading the file while the terminal runs. That is
  [the file observer plan](desktop-configuration-file-observer-plan.md), which predates ADR 0013
  and needs rewriting before it can be picked up.
- Migrating an existing `desktop.json`. ADR 0037 decides against it.
