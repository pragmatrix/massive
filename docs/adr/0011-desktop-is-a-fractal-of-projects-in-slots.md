# The desktop is a fractal of projects in slots

## Status: accepted

The desktop previously had exactly two levels: the `Desktop` (the implicit root
that fit all projects) holding top-level projects, each project owning a matrix
of launchers. The model now recurses: exactly one project is the root, and
`FocusDepth::Desktop` is removed — the root project's `Project` depth is the
outermost Focus Depth. A project's matrix
is filled by **slots**, and each slot hosts either a launcher or another nested
project — never both. Nested projects are single-parented and appear in exactly
one placement. A launcher keeps its existing meaning: the configured profile
that spawns instances and owns a launcher mode. Nesting is arbitrary in depth.

The `DesktopTarget::Desktop` **target** returns as the hierarchy's root: a
virtual node that is never inserted explicitly, has no presenter, and exists
only as the parent key under which the root project's target hangs. The root
project itself is one special nested project — id [`ProjectId::ROOT`], a fixed
constant so the boot command can name it before the system exists — created by
the boot flow's first command (`AddProject { under: None }`) through plan and
transact, like every other project.

Slots are **implicit**: a slot is a matrix slot of a project, present only
while a launcher or a nested project is assigned to it, and addressed by its
*(project, placement)* key. There is no `SlotId` and no slot node in the
topology. A cleared placement is simply empty, so a project may have no
slots at all — its contents are what exist.

## Presentation and interaction are one depth cut (deferred)

This rule is not implemented yet: the first pass renders every project's content
and hit-tests whatever is rendered, so nothing is pruned and no project is
limited to one slot level. The rule stands as the target, and the mechanisms
below are its design.

A project that is not the focused project presents exactly one slot level deep:
its slots render (a launcher slot without its instances, a project slot without
its nested slots) and the slots are only clickable — a click is a zoom-in
gesture. Nothing deeper is rendered or interactive. The focused project itself
behaves as before at every Focus Depth. Rendering (content pruning) and
interaction (hit testing and focus routing) share this single rule, which is
what makes the model fractal rather than two ad-hoc behaviors; it generalizes
to arbitrary nesting depth without new vocabulary. Only the Focus Depth
borrows the slot word. The overview Focus Depths read outermost first —
`Project, Row, Slot, Instance, Full Screen` (`Launcher` renamed to `Slot`,
`Desktop` removed): a Focus Depth's position counts the zoom-ins from the project
level, so the depths compose across nesting — each zoom-in through a project
slot switches to the newly focused project's Focus Depth sequence. There is no
depth beyond the root project's Project depth, and `ZoomOut` there is a no-op
(clamped; repr `0` makes it the underflow guard).

*Update:* the `Full Screen` depth is superseded by
[ADR 0014](0014-fullscreen-mode-is-per-launcher-content-scaling.md) — the
Focus Depths read `Project, Row, Slot, Instance`.

## Nested projects render scaled, at full layout size

A nested project is laid out at its own full size — identical layout code at
every level, no reflow per slot — and its slot presents that scene uniformly
**scaled down** to fit the slot rect. This is a content presentation scale on
the nested scene, deliberately distinct from the camera mechanism: ADR 0004
removes *model scale from the camera* because a scale/distance two-channel
product makes zoom transitions non-monotonic; a nested scene's presentation
scale is not a camera channel, the camera still dollies only in distance, and
when the nested project is the focused project its content presents at scale 1,
so text shapes pixel-perfect at the native size. Zooming into a project slot
animates the camera toward the slot while the presentation scale eases to 1.
Because layout is slot-independent, there is no minimum shrink floor: a deep
nested project may preview below legibility, and legibility returns by zooming
in.

The preview's *size* is not derived from the nested scene: a project-assigned
slot measures like an instance panel (`default_panel_size`), and the scale is
that slot rect over the nested project's own size. The scale lives in the slot's
placement transform, where `set_layout` already carries it (`SizedTransform`),
and `absolute_placement` propagates it to everything below — hover rects and
hit-test transforms scale with the content, and the nested project's own
presenter applies no second scale.

## Vocabulary and addressing

Configuration addressing uses **address paths**: slash-separated names resolved
nearest-match per segment (ADR 0010's duplicate-name rule, applied per level).
The invoking instance's project is the implicit base for relative paths from the
CLI; a leading separator addresses from the root. `startup` persists an address
path.

The configuration change vocabulary is slot-centric and **assign/clear**
shaped, because a slot may be empty:

- `AssignSlot` — the parent project, a placement, and the content: a launcher
  profile or a project name. Assigning a project name creates that nested
  project (empty) and places it.
- `AddProject { under: None }` — creates the root project; only the root is
  created parentless, and only once (the plan rejects a second attempt).
- `ClearSlot` — the parent project and a placement.
- `MoveSlot` — a source *(parent, placement)* and a destination; the content may
  move to another project's matrix, but not into its own subtree.
- `SetStartup` — the startup launcher's address path.

Replacing slot content is a clear plus an assign, never an in-place operation.
Displacement is not part of the change vocabulary: a `SlotShift` policy rides
the *plan* (as the removal shift does today) and expands into concrete
`MoveSlot`s before anything is applied, so the live model and the document
mirror both receive deterministic changes.

## The file is flat; the root is synthesized

The configuration document does not name the root. Its top-level `launcher` and
`project` entries are the root project's slots, and nesting is expressed by
nested `project` nodes; the terminal creates the root ([`ProjectId::ROOT`], named
`Projects`) when the document is loaded — the boot flow's first command
re-applies it, and the parse pre-builds it in the aggregate because the slot
assignments derive from it. (The document format is now JSON, derived from the
aggregate — see [ADR 0013](./0013-desktop-configuration-is-json-derived-from-the-aggregate.md).) Migration of a file written before this decision is a parse-time step: every
former top-level project becomes a root slot at `column=0, row=<document index>`
— preserving the vertical order the old desktop laid them out in — with its own
launchers untouched inside it. The file is rewritten only on the first
configuration change, so a session that changes nothing leaves it as it was and
every boot re-derives the same tree.

## Boot still requires a launcher

Empty projects and empty slots are legal, but the session still boots into a
launcher-backed primary instance, so a configuration with no launcher anywhere
in the tree is rejected at load. The startup launcher's address path falls back
to the nearest depth-first launcher from the root when it does not resolve.

## Consequences

- The topology helpers that type-enforced launcher-only matrices
  (`launcher_of_instance`, `project_of_launcher`, `matrix_launchers`,
  `launcher_instances`) classify slot content instead of assuming it.
- `project_of_target` answers the focused project's id for every target — one
  exception being documented: `Desktop` goes *down*, not up, resolving to the
  root project (a hit-test miss produces a `Desktop` target), and the exception
  arm logs, so situations that reach it stay observable.
- `project_of_target` answering the root for every target keeps the derived
  focused project and the depth sequence dependent on a project being found
  at the root.
- Rect math reads the placement rect (origin space), not the placement
  transform's scale: `to_origin_space`/`to_anchor_space` only round-trip at
  scale 1. This includes the overview bounds and the row/project rect widening.
