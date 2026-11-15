# The desktop configuration is KDL, reconciled from the aggregate

## Status: accepted, implemented

Amended by [ADR 0047](../0047-a-watcher-applies-desktop-configuration-edits-at-once.md) (proposed):
once a watcher applies edits of the file at once, the persistence keeps the document in memory and
no longer reads the file at each persist.

Supersedes the JSON format of
[ADR 0013](./0013-desktop-configuration-is-json-derived-from-the-aggregate.md).
ADR 0013's central decision stays: the document is derived from the aggregate at
every persist, and there is no separate edit path.

Amended by [ADR 0044](./0044-launchers-present-only-as-visors.md): a launcher has no `mode`, so the
format neither writes nor reads one.

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
| A hand-written mapping between the document and the aggregate. | Yes. `kdl` 6.7.1 has serde support, but its mapping cannot read this format (see the considered options). |

## Decision

The desktop configuration is stored as KDL in `~/.massive/desktop.kdl`.

- **Persisting reconciles.** At every persist, the aggregate is turned into a
  fresh KDL document, and the fresh document is reconciled into the document
  parsed from the file on disk. The result is written atomically. The reconcile
  is one pure function from the existing and the fresh document to the document
  that is written. Nothing is kept between persists: the file is read and parsed
  again each time, so hand edits made while the terminal runs are not lost.
- **The reconcile keeps what still matches.** A slot node matches a sibling in
  the existing document in three passes: by kind, slot position, and name, then
  by kind and name, then by kind and slot position. A moved slot, even one
  swapped with another, so keeps its comments and, for a project, its whole
  subtree, and a slot renamed in place keeps them too. The `startup` node and
  the `mcp` node match by their node name, and a spawn parameter matches by its
  key. A matched node keeps its comments and
  formatting. When a matched node's values differ, all of its values are replaced
  by the fresh ones. Values cannot be updated in place, because
  `KdlEntry::set_value` keeps printing the old value's text. An existing node
  without a match is removed together with the comments above it. A fresh node
  without a match is inserted at its fresh position. The written document
  follows the order of the fresh document, so it reads as exactly the fresh
  document. The fresh document is
  autoformatted and parsed again before the reconcile, so an inserted node
  already has the indentation of its depth.
- **The comments at the top of the file stay there.** The parser attaches them
  to the first node, so when a blank line separates them from that node, the
  reconcile moves them to the document itself. They then stay when the first
  node is removed or another node comes first. The header ends only at a blank
  line before which the text parses on its own, so a blank line inside a block
  comment does not split it.
- **The format follows the fractal view of ADR 0011.** The root project's slots
  are the top-level nodes. A `project` node and a `launcher` node take the name
  as their first value and the slot position as `at`. A project's slots are its
  child nodes, and a launcher's child nodes are its spawn parameters. Default
  values are omitted: `full_screen_mode` is written only for `fullscreen`.

  ```kdl
  startup "/ffm + ber/fs110"

  project "ffm + ber" at="0,2" {
      // The frontend servers.
      launcher fs110 at="0,0" {
          command "ssh fs110"
      }
      launcher build at="1,0" full_screen_mode=fullscreen
  }
  ```

- **A slot position is the string `at="column,row"`.** KDL has no pair value,
  and `kdl` 6.7.1 rejects the unquoted forms `at=0,0` and `at=0:0`. The parser
  splits the string and validates both numbers. A malformed position fails the
  load with the file path in the message, like any other parse error.
- **An MCP root (ADR 0036) carries an `mcp` child node.** The node's arguments
  are the access scopes, for example `mcp observe act`. An MCP root without
  access scopes is the bare node `mcp`. A project without an `mcp` node is not
  an MCP root.
- **A spawn parameter is a child node of its launcher, named by its key, with
  its value as its one argument.** The value is a string, a number, a boolean,
  or `#null`, as in `command "ssh fs110"`. A param node with no argument,
  several arguments, a property, or a children block fails the load. The MCP
  tools refuse a param that is an array or an object. Params are plain values
  because the only param any application reads is the terminal's `command`, a
  string. JSON arrays and objects do not map onto KDL nodes without a shape of
  their own for each case. A one-element array, for example, reads back as its
  element.
- **Every string is written.** `kdl` escapes only quotes, backslashes, and a few
  control characters, and it writes the other characters KDL forbids in its
  text as they are, such as ESC, U+2028, the direction marks, and U+FEFF. It
  then cannot parse its own output. The writer escapes those characters as
  `\u{…}` in names, param keys, and string values, so no name or param a tool
  accepts makes a persist fail.
- **The load is strict.** An unknown node or property fails the load, with the
  file path, line, and column in the message. Otherwise the reconcile would find
  no match for the unknown node and remove it, together with its comments. A
  repeated property and a type annotation fail the load too, because nothing
  reads them.
- **A file that does not load is not overwritten.** When the file on disk fails
  to parse or to load strictly at a persist, the persist writes nothing and logs
  an error that says the changes are not saved until the file loads again. A
  half-finished hand edit therefore survives. The next persist writes the whole
  aggregate again, so no change is lost once the file loads. The persist does
  not fail the transaction, because only an edit outside the desktop breaks the
  file, and a typo in it must not end the session. The persist checks the file
  with the same check as the load, including the nesting limit.
- **The written text is checked before it is written.** The persist parses the
  reconciled text in memory and writes it only when it reads as exactly the
  fresh document. Otherwise it writes nothing and logs an error, because a bug
  in the reconcile or in `kdl` would otherwise reach the file.

## Consequences

- The reconcile and the format live inside the persistence module. The aggregate
  (`RuntimeConfiguration`), the command dispatch, and the configuration changes
  do not change.
- The mapping between the document and the aggregate is written by hand, in both
  directions.
- Params need no nesting limit, so `MAX_PARAMS_DEPTH` and its policy rule go.
  An application that needs a list or a map in its params gets a design of its
  own, typed by that application, once one exists.
- Hand edits to values made while the terminal runs are replaced at the next
  persist, because the aggregate in memory wins. Only comments and formatting
  survive. Reloading the file while the terminal runs is left to
  `docs/desktop-configuration-file-observer-plan.md`.
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
- **`kdl`'s serde support (`kdl::de`, `kdl::se`).** Rejected after a small
  experiment. It reads a single `project` node as a scalar where the struct
  expects a sequence. A `#0` field for the name argument breaks beside a
  flattened params map. Unknown nodes are ignored without an error, so the
  reconcile would remove them.
- **Applying the difference between the aggregate before and after a
  transaction.** Rejected for now. Ids would identify a moved slot exactly,
  where the reconcile pairs moved nodes by name. But the persistence would keep
  the previous aggregate between persists, and it would find nodes in the file
  by their old paths, which a hand edit invalidates. It can later replace the
  reconcile's pass by name without a change to the format.
- **Params as any JSON value, with nested objects as nested child nodes.**
  Rejected after review. Each JSON shape needs a node shape of its own: an
  array is several arguments, an object a children block, and an empty object
  `{}`. A one-element array cannot be told apart from its element without one
  more shape, and an array inside an array needs yet another rule. No
  application reads any of these.
- **Typed params, declared by each application.** Not now: the reader and the
  tools would validate against a schema per application. That pays off once an
  application reads a second param.
- **The access scopes as a property (`mcp="observe act"`) or as a nested node
  (`mcp { access_scopes observe act }`).** Rejected: the property needs
  `mcp=""` for an MCP root without scopes, and the nested node adds a level
  that no second MCP setting needs yet.
