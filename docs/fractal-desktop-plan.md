# Fractal desktop — implementation plan

Derived from the grilling session (glossary in `CONTEXT.md`, decision record in
`docs/adr/0011-desktop-is-a-fractal-of-projects-in-slots.md`). Ordered so every
step compiles.

**Deferred from ADR 0011 to a later pass**: the depth cut (pruning everything
below a non-focused project's slot level, and its fade) and the one-slot-deep
hit-test rule. Until they land, every project renders all of its content and hit
testing reaches whatever is rendered.

## 1. Model: slots in the runtime tree

The runtime tree keeps content targets (`Project`, `Launcher`, `Instance`,
`View`) but loses the implicit two-level assumption: a `ProjectMatrix`'s
children become a slot-disciplined sequence. Four panics in `topology.rs`
(`launcher_of_instance`, `project_of_launcher`, `matrix_launchers`,
`launcher_instances`) type-enforce the two-level shape and must generalize: the
matrix returns ordered **slot entries**, each classifying its content as
`Launcher(LaunchProfileId)` or `Project(ProjectId)`. Call sites (layout,
navigation, removal planning, boot) consume slot entries instead of assuming
the cell kind from the parent.

Key sub-points that the scan of the current code surfaced:

- **Elaboration (decided): slot entries, not a `Slot` topology node.** No
  `SlotId` namespace, no third id alongside `ProjectId`/`LaunchProfileId`. The
  slot is *(matrix placement → content)*, addressable by
  `(parent ProjectId, MatrixPlacement)` — the same key the shift planner uses
  today. What a slot needs beyond the placement is a *presenter target it can
  point at*: hover outline and click-to-zoom route only
  `Launcher | Instance | View` targets (`presentation.rs`), so the click/hover
  target for a project-occupied slot is the nested `Project` target itself —
  which already exists as `DesktopTarget::Project(_)` and already has
  camera-resolution entries (`Project*` → project depth). Launcher-occupied
  slots keep targeting the launcher. This keeps `DesktopTarget` unchanged
  except for dropping `Desktop`.
- **Shifts stay content-agnostic per cell** (decided: project slots shift like
  launchers), which is exactly the per-cell shape the existing shift code
  already has — it generalizes from launchers to slot entries by placement, not
  by needing a cell id.
- **`matrix_launchers`' zip-with-children order contract**: the layout
  algorithm zips `matrix_launchers(project)` against the matrix's children
  list; the classification of slot entries must therefore preserve the same
  child-order contract or the zip becomes a placement-keyed join.
- **Aggregates**: `Aggregates` keeps flat `projects`/`launchers`/`instances`
  maps (unchanged); the matrix content classification is derived, not stored —
  a cell's content kind is discoverable from the configuration aggregate
  (`Slot { placement, content }`, step 2) or from the topology parent kind
  itself (`DesktopTarget::Project(id)` under a matrix *is* the slot content).
  No extra runtime state is created for slots.
- **Presenters**: no separate `SlotPresenter` kind — a launcher-occupied slot
  is presented by the existing `LauncherPresenter` (its visor already renders
  the cell), and a project-occupied slot is presented by the nested
  `ProjectPresenter` scaled down (step 5). An unoccupied cell has no content,
  so it presents as nothing.
- Topology rules in `topology.rs`: a slot's content is `Project` or `Launcher`,
  mutually exclusive; root = exactly one project, and the root is the single
  parentless target.
- Test fixtures encode the two-level shape (Desktop-rooted project lists in
  `layout_state.rs`, `navigation.rs`, `matrix_navigation.rs`,
  `hierarchy_focus.rs` test modules) — step 4 includes wrapping them in the
  root project, else the build fails mid-step.

## 2. Configuration aggregate and change vocabulary

- `configuration.rs`: `Project { id, name, slots: Vec<Slot> }`, `Slot {
  placement, content: SlotContent }`, `SlotContent = Launcher(LaunchProfileId)
  | Project(ProjectId)`. Projects stay a **flat** `Vec<Project>` in
  `DesktopConfiguration` — nesting is expressed by id links plus a parent
  lookup, so `project(id)`/`launcher(id)` stay O(n) and no recursive type is
  needed. Empty cells are simply absent; a project with no slots, and a
  configuration whose launchers all sit in nested projects, are both legal.
- **No `AddProject`.** Creating a nested project *is* assigning a
  `SlotContent::Project(name)` to a slot: it creates the project (empty) and
  places it. The root is not created by a command at all (step 4).
- `ConfigurationChange` (ADR 0011's vocabulary, content as a field):
  `AssignSlot { parent: ProjectId, placement, content: SlotContent }`,
  `ClearSlot { parent: ProjectId, placement }`, `MoveSlot { source: (ProjectId,
  MatrixPlacement), dest: (ProjectId, MatrixPlacement) }`, `SetStartup { target
  }`. `ProjectCommand` is renamed to the same vocabulary, as is
  `DesktopChange`'s `Project(ConfigurationChange)` wrapper.
- **The shift policy is a planning concern, not a persisted change.** Like
  today's `RemoveSlotShiftingPolicy`, a `SlotShift::{Shift, Keep}` rides the
  request and the `plan_*` call and expands into concrete `MoveSlot` changes
  before anything applies: `Shift` on assign moves the occupant and its
  contiguous run one column right (today's `AddLauncher`), `Shift` on clear
  pulls the same row left (today's `ShiftLeft`); `Keep` on assign replaces the
  occupant (recursively removing its subtree), `Keep` on clear leaves a gap.
  `ConfigurationChange` stays policy-free, so the live model and the document
  mirror both receive deterministic concrete changes.
- `MoveSlot` may cross parent matrices; a move into the moved subtree's own
  descendant, into a project not in the tree, or off the matrix edge is
  rejected with no change emitted.
- **No cascade**: removing the last slot of a project does not remove the
  project (empty projects are legal). The invariant that remains is the one
  boot depends on: **at least one launcher must exist in the tree**, because
  the session boots into a launcher-backed primary instance.
- Startup resolves by address path; unresolved at load falls back per segment to
  the nearest match, then to the root's first launcher by depth-first walk, with
  a warning.
- Undo/Redo stay `todo!()` — deferred until after the model works; the slot
  change vocabulary must not grow inverses for them yet.

## 3. KDL document and migration

- The document stays **flat**: top-level `launcher` and `project` nodes are the
  *root project's* slots, and nesting is expressed by nested `project` nodes.
  The root has no node — the terminal synthesizes it (step 4) — so a
  hand-written file never mentions it:

  ```kdl
  launcher "dev" column=0 row=0 mode=band { spawn "…" }
  project "labs" column=1 row=0 {
      launcher "shell" column=0 row=0
  }
  startup "/labs/shell"
  ```

- **Migration of an old file**: every former top-level `project` becomes a root
  slot, its own launchers keeping their placements inside it. The synthesized
  root placements preserve today's vertical order — `column=0, row=<document
  index>`. Old `startup "name"` resolves against the migrated tree, else falls
  back.
- Nodes are tagged recursively (ADR 0010's in-memory tag in the node span);
  `launcher_node_of`/`remove_project`/`add_launcher` currently assume one child
  level or root-level projects and become recursive tag lookups. An empty
  `project "x" { }` node parses to a project with no slots.
- The parser must recognize nested `project` nodes: it currently warns and
  drops them ("Ignoring unknown node" inside a project), so any nested file
  silently loses its subtrees until the parser learns the node kind.
- Boot re-application (`to_commands`) becomes a **pre-order DFS** from the root:
  today it emits every project flat under the Desktop; with nesting, a slot's
  parent project (and its matrix) must exist before the slot is assigned under
  it. The primary-instance choice (`boot_launcher`) becomes a depth-first walk
  from the root, so a root whose slots are all nested projects still boots.
- Migration is a parse-time transformation of both the aggregate and the
  in-memory document; the file itself is only rewritten on the first
  configuration change (Setup does not flush — existing behavior), so a
  session that changes nothing leaves the file untouched and every boot
  re-derives the migration idempotently.

## 4. Root project; the `Desktop` sentinel is removed

- `DesktopSystem::new` creates the root project (its name is `Projects`) and
  its presenter; the root is the single parentless target.
- Remove `DesktopTarget::Desktop` / `FocusDepth::Desktop` matches — the
  enumerated sites: indicator labels, `zoom_navigation.rs`'s desktop camera
  arm **and** `with_desktop_width`, `focus_depth_indicator.rs`,
  `focus_input.rs` floor clamps, `command.rs` docs, plus these semantic
  special-cases: the layout-space root in `place_children_of` /
  `absolute_placement` (layout root = parentless target, not the Desktop
  sentinel), the hit-test root and its origin-space branch, the root
  `LayoutSpec` wrapper, `project_of_target`/focus-path root arms (see step
  6), `ResizeAll`/`WindowResized` root measure targets, and the project
  planner's `parent_target` (step 2).
- The root renders exactly like a nested project: header + `PROJECT_PADDING`,
  `PROJECT_HEADER_SPACING` between header and matrix. `SECTION_SPACING`
  disappears.

## 5. Layout: full-size nested scenes, scaled presentation

- The layout algorithm is unchanged at single-project scale; a nested project
  is laid out at its own full size. A project-occupied slot **measures like an
  instance panel** (`default_panel_size`), and the nested project presents
  uniformly scaled to fit that slot rect — the scale is the slot rect over the
  nested project's own size, *not* via `PixelCamera` (ADR 0004: camera
  dolly-only).
- The scale is applied once. It rides the layout transform: `set_layout`
  already carries a full `SizedTransform` and `Transform::mul` composes
  scales, so the slot's placement applies the scale in the same channel it
  already uses for position/size, and `absolute_placement` propagates it to
  everything below — hover rects and hit-test transforms scale with it for
  free. The nested project's presenter must not apply it a second time.
- Rect math derives from `placement.rect` (origin space), never from
  `transform.scale`: `to_origin_space`/`to_anchor_space` only round-trip at
  scale 1. `target_rect`/`project_rect`/`matrix_row_rect` therefore read rects
  that already carry the presentation scale, and `with_desktop_width` (Row /
  Project rect widening for cross-project panning) re-derives its extent from
  the focused project's parent-matrix width; the root extends to its own width.

## 6. Focus depth and navigation

- `FocusDepth`: `{Project, Row, Slot, Instance, InstanceFullScreen}` — read
  outermost first (rename `Launcher`→`Slot`, drop `Desktop`), so the `u8`
  discriminant counts zoom-ins from the project level; `ZoomOut` at root
  Project floor clamps (no-op; repr `0` makes the floor the underflow guard).
- **No stored focus context (decided):** the focused project is *derived* from
  the currently focused target by walking the ancestor chain to the nearest
  project in the topology — the depth ladder stays relative to that project
  and the existing single `focus_depth` field carries the rung. Entering a
  nested project is a focus change (its target resolves to the project depth
  via the existing nearest-depth walk), not a state change.
- `zoom_in` at Slot depth branches on slot content: launcher → `Instance`,
  project → focus the nested project (entering at its `Project` depth).
- `ZoomOut` from a nested project lands at the parent's Slot depth; the
  focused project re-derives to the parent.
- `project_of_target` at the root answers the root project's id (not `None`)
  — the desktop arm disappears; the derived focused-project walk depends on
  this at the ladder floor, and the camera walk's `expect` becomes the repr-0
  floor.
- Keyboard navigation: navigating onto a project slot focuses the nested
  `Project` target (the focused project re-derives); cross-project vertical
  overflow becomes sibling-slot navigation through the *parent* slot's sibling
  sequence (column affinity semantics unchanged); a `Project` target becomes a
  navigation origin.
- Camera resolution: the camera fits the *scaled* rect of a nested
  presentation directly, like any other content — no composed per-level
  camera math. Focused-project cameras per existing per-depth code.
- Indicator: 5 labels, `Project / Row / Slot / Instance / Full Screen` (the
  label array is indexed by repr, so it must match the new declaration order).

## 7. CLI / requests (`massive-terminal` `src/command.rs`,
`src/terminal_request.rs`, `applications/src/instance_environment.rs`)

- `ConfigurationRequest` unifies assign with a content field:
  `AssignLauncher { name, placement, under: Option<Path> }`,
  `AssignProject { name, placement, under: Option<Path> }` (assigning a
  project creates the nested project), `ClearSlot { name: Option<String> }`
  (nearest-match; both `remove launcher` and `remove project` lower to it),
  `MoveSlot { direction }`, `PushSlot { direction }`,
  `SetStartup { path: Option<String> }`, plus `Resize`/`Undo`/`Redo`
  unchanged. Requests carry the `SlotShift` policy, defaulting to `Shift`.
- Address paths: `labs/shell` is relative to the invoking instance's project;
  a leading `/` addresses from the root, resolved nearest-match per segment.
- `mt` subcommands: `add project NAME COL ROW [--under PATH]`,
  `add launcher NAME COL ROW [--under PATH]`,
  `remove launcher|project --name`, `move`/`push` (with a cross-matrix
  destination), `startup [PATH]`.

## 8. Documentation

- Amend `docs/adr/0011-desktop-is-a-fractal-of-projects-in-slots.md` and
  `CONTEXT.md` to the decisions here: implicit slots and legal empty projects,
  the assign/clear vocabulary with the planning-layer shift policy, the flat
  file with a synthesized `Projects` root, the `default_panel_size` preview,
  and the deferral of the depth cut and the hit-test rule.

## Risks / watch-outs

- `project_matrix_tracks`/`place_project_matrix_children` zip
  `matrix_launchers` with child sizes; a project slot's cell size must come
  from `default_panel_size`, or the nested project's own measurement becomes
  the cell size and the scale collapses to 1.
- The presentation scale must be applied once (by the matrix placement), or
  the nested project's presenter doubles it.
- The camera walk's `expect` and the hit-test miss both need the root to answer
  as a target; the hit-test miss currently returns `Desktop`.
- `transform`-derived rects are wrong at scale != 1 (see step 5), including the
  overview bounds math in `zoom_navigation.rs`.
- The document mirror's `NodeTags` lookups assume one child level and
  root-level projects.
- `to_commands` must emit parent-before-child and `boot_launcher` must reach
  nested projects.

## Open issues

1. **Document-mirror ordering under multi-change operations** — the document
   mirror lands before the live model and a failing transaction leaves earlier
   effects applied (transact documents itself as "not a transaction yet"). With
   the removal cascade gone, the remaining multi-change operations are the
   assign-into-occupied-cell shift and the startup auto-rewrite. Decide whether
   the mirror applies after the live model before those land.
