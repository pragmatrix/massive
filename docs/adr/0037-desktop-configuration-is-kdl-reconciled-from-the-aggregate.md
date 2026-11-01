# The desktop configuration is KDL, reconciled from the aggregate

## Status: accepted, not yet implemented

Supersedes the JSON format of
[ADR 0013](./0013-desktop-configuration-is-json-derived-from-the-aggregate.md).
ADR 0013's central decision stays: the document is derived from the aggregate at
every persist, and there is no separate edit path.

## Problem

The JSON file is hard to read and cannot hold comments. `serde_json`'s pretty
printer puts every value on its own line, so a configuration of three projects
and thirteen launchers takes 198 lines. Each `[column, row]` pair alone takes
four of them. The file is meant to be edited by hand, and a hand-edited file
needs comments that survive the terminal's own writes.

ADR 0013 left KDL because of the machinery around the surgical edits of
[ADR 0006](./0006-desktop-configuration-is-name-keyed-kdl.md), not because of
the format. That implementation took about 1,220 lines of persistence code
(`document.rs`, `configuration.rs`, and `parameters.rs`, removed in massive
commit `db68925c`). Most of it followed from editing the document alongside the
aggregate, and a document derived from the aggregate does not need it:

| Machinery of the old KDL implementation | Needed with a document derived from the aggregate? |
|---|---|
| A separate document edit for each kind of configuration change (`set_startup`, `assign_slot`, `clear_slot`, `move_slot`, and a migration), mirrored beside the aggregate. | No. One reconcile handles every kind of change, including later additions such as the Full Screen Mode of ADR 0014. |
| Node identity tags riding in the parser's `span` field (ADR 0010). | No. A node is identified by its kind and slot position among its siblings, which are unique within a project. |
| The risk that the document and the live aggregate drift apart. | No. The file is derived from the aggregate, and the reconciled result parses back to exactly the fresh document. |
| Formatting bookkeeping for inserted nodes (`fresh_node_format`, `whitespace_leading`, and the fixes that kept a new node from copying its sibling's comments). | Mostly no. The fresh document is autoformatted and parsed again, so each new node already has the indentation of its depth. Blank lines between top-level projects are not produced. |
| The conversion of spawn parameters between JSON values and KDL child nodes. | Yes, unchanged. |
| A hand-written mapping between the document and the aggregate, because `kdl` has no serde support. | Yes. |

## Decision

The desktop configuration is stored as KDL in `~/.massive/desktop.kdl`.

- **Persisting reconciles.** At every persist, the aggregate is turned into a
  fresh KDL document, and the fresh document is reconciled into the document
  parsed from the file on disk. The result is written atomically. The reconcile
  is one pure function from the existing and the fresh document to the document
  that is written. Nothing is kept between persists: the file is read and parsed
  again each time, so hand edits made while the terminal runs are not lost.
- **The reconcile keeps what still matches.** A node matches a sibling in the
  existing document by its kind and slot position. The `startup` node matches by
  name, and a spawn parameter matches by its key. A second pass pairs the nodes
  left over on both sides by kind and name, so a moved slot keeps its comments
  and, for a project, its whole subtree. A matched node keeps its comments and
  formatting. When a matched node's values differ, all of its values are replaced
  by the fresh ones. Values cannot be updated in place, because
  `KdlEntry::set_value` keeps printing the old value's text. An existing node
  without a match is removed together with the comments above it. A fresh node
  without a match is inserted at its fresh position. The fresh document is
  autoformatted and parsed again before the reconcile, so an inserted node
  already has the indentation of its depth.
- **The format follows the fractal view of ADR 0011.** The root project's slots
  are the top-level nodes. A `project` node and a `launcher` node take the name
  as their first value and the slot position as `at`. A project's slots are its
  child nodes, and a launcher's child nodes are its spawn parameters. Default
  values are omitted: `mode` is written only for `band`, and `full_screen_mode`
  only for `fullscreen`.

  ```kdl
  startup "/ffm + ber/fs110"

  project "ffm + ber" at="0,2" {
      // The frontend servers.
      launcher fs110 at="0,0" {
          command "ssh fs110"
      }
      launcher build at="1,0" mode=band
  }
  ```

- **A slot position is the string `at="column,row"`.** KDL has no pair value,
  and `kdl` 6.7.1 rejects the unquoted forms `at=0,0` and `at=0:0`. The parser
  splits the string and validates both numbers. A malformed position fails the
  load with the file path in the message, like any other parse error.

## Consequences

- The reconcile and the format live inside the persistence module. The aggregate
  (`RuntimeConfiguration`), the command dispatch, and the configuration changes
  do not change.
- `kdl` has no serde support. The mapping between the document and the aggregate
  is written by hand, in both directions.
- Spawn parameters are JSON values (`Params`), and a KDL node holds only scalars.
  A parameter becomes a child node with its value as the argument, and an array
  becomes several arguments. A nested object cannot be written. The old KDL
  implementation logged and skipped such a value, and the new one does the same.
- A node whose values changed loses the formatting of its values, such as quotes.
  `kdl` 6 writes KDL v2, which leaves plain strings unquoted, for example
  `launcher fs110`.
- Node identity tags (ADR 0010) do not return. Duplicate names stay legal, because
  a node is identified by its slot position first. The pass that matches by name
  sees only the nodes that position could not pair.
- The `kdl` dependency returns to the desktop crate. mt's terminal settings
  already use it, so the build gains no new dependency.
- An existing `desktop.json` is not migrated automatically, for ADR 0013's
  reason: there are no other users.

## Considered options

- **Keep JSON and use JSONC with `jsonc-parser`'s comment-preserving syntax tree.**
  Rejected: it keeps comments, but the syntax stays noisy, and the crate writes
  every inserted object across several lines. One launcher added by the terminal
  becomes a six-line block.
- **TOML with `toml_edit`.** Rejected: nested projects need repeated
  `[[slots.project.slots]]` headers or deeply nested inline tables. Duplicate names
  (ADR 0010) rule out tables keyed by name.
- **YAML.** Rejected: `serde_yaml` is deprecated, and the only comment-preserving
  editor found, `yaml-edit`, is young.
- **RON or JSON5.** Rejected: no Rust crate found writes either format back with its
  comments, so every persist would drop them.
- **Surgical edits per change, as in ADR 0006.** Rejected, for ADR 0013's reasons.
- **The slot position as two numbers after the name (`launcher fs110 0 0`).**
  Rejected: it is the shortest form, but the numbers' meaning comes only from their
  place.
- **The slot position as `column=0 row=0`, as before.** Rejected: it explains
  itself but is the longest form, and it repeats on every slot.
