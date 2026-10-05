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
the slot content kind from the parent.

Key sub-points that the scan of the current code surfaced:

- **Elaboration (decided): slot entries, not a `Slot` topology node.** No
  `SlotId` namespace, no third id alongside `ProjectId`/`LaunchProfileId`. The
  slot is *(matrix placement → content)*, addressable by
  `(parent ProjectId, MatrixPlacement)` — the same key the shift planner uses
  today. What a slot needs beyond the placement is a *presenter target it can
  point at*: hover outline and click-to-zoom route only
  `Launcher | Instance | View` targets (`presentation.rs`), so the click/hover
  target for a project-assigned slot is the nested `Project` target itself —
  which already exists as `DesktopTarget::Project(_)` and already has
  camera-resolution entries (`Project*` → project depth). Launcher-assigned
  slots keep targeting the launcher. This keeps `DesktopTarget` unchanged, and
  restores the pre-fractal `Desktop` target as the hierarchy's virtual root
  (step 4).
- **Shifts stay content-agnostic per slot** (decided: project slots shift like
  launchers), which is exactly the per-slot shape the existing shift code
  already has — it generalizes from launchers to slot entries by placement, not
  by needing a slot id.
- **`matrix_launchers`' zip-with-children order contract**: the layout
  algorithm zips `matrix_launchers(project)` against the matrix's children
  list; the classification of slot entries must therefore preserve the same
  child-order contract or the zip becomes a placement-keyed join.
- **Aggregates**: `Aggregates` keeps flat `projects`/`launchers`/`instances`
  maps (unchanged); the matrix content classification is derived, not stored —
  a slot's content kind is discoverable from the configuration aggregate
  (`Slot { placement, content }`, step 2) or from the topology parent kind
  itself (`DesktopTarget::Project(id)` under a matrix *is* the slot content).
  No extra runtime state is created for slots.
- **Presenters**: no separate `SlotPresenter` kind — a launcher-assigned slot
  is presented by the existing `LauncherPresenter` (its visor already renders
  the slot), and a project-assigned slot is presented by the nested
  `ProjectPresenter` scaled down (step 5). A cleared slot has no content,
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
  needed. Empty slots are simply absent; a project with no slots, and a
  configuration whose launchers all sit in nested projects, are both legal.
- **`AddProject { under: None }` exists only for the root.** Creating a nested
  project *is* assigning a `SlotContent::Project(name)` to a slot: it creates
  the project (empty) and places it. The root is the one parentless creation,
  guarded by the plan (step 4).
- `ConfigurationChange` (ADR 0011's vocabulary, content as a field):
  `AssignSlot { parent: ProjectId, placement, content: SlotContent }`,
  `ClearSlot { parent: ProjectId, placement }`, `MoveSlot { source: (ProjectId,
  MatrixPlacement), dest: (ProjectId, MatrixPlacement) }`, `SetStartup { target
  }`. `ProjectCommand` is renamed to the same vocabulary, as is
  `DesktopChange`'s `Project(ConfigurationChange)` wrapper.
- **The shift policy is a planning concern, not a persisted change.** Like
  today's `RemoveSlotShiftingPolicy`, a `SlotShift::{Shift, Keep}` rides the
  request and the `plan_*` call and expands into concrete `MoveSlot` changes
  before anything applies: `Shift` on assign moves the assigned content and its
  contiguous run one column right (today's `AddLauncher`), `Shift` on clear
  pulls the same row left (today's `ShiftLeft`); `Keep` on assign replaces the
  assigned content (recursively removing its subtree), `Keep` on clear leaves a gap.
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
  it starts with the root project's own `AddProject { under: None }`, then a
  slot's parent project (and its matrix) must exist before the slot is assigned
  under
  it. The primary-instance choice (`boot_launcher`) becomes a depth-first walk
  from the root, so a root whose slots are all nested projects still boots.
- Migration is a parse-time transformation of both the aggregate and the
  in-memory document; the file itself is only rewritten on the first
  configuration change (Setup does not flush — existing behavior), so a
  session that changes nothing leaves the file untouched and every boot
  re-derives the migration idempotently.

## 4. Root project; the `Desktop` target returns as the virtual hierarchy root

- `DesktopTarget::Desktop` is restored as the hierarchy's root — a *virtual*
  node: never inserted up front, no presenter, materialized implicitly as the
  parent key of the root project's `TopologyChange::Add`. `FocusDepth::Desktop`
  stays removed; the root project's `Project` depth is the outermost Focus
  Depth.
- The root project is one special nested project with a **fixed id**
  (`ProjectId::ROOT`, the nil UUID): created by the boot flow's first command,
  `AddProject { id: ProjectId::ROOT, name: "Projects", under: None }` through
  plan and transact; `DesktopSystem::new` no longer creates it. The plan
  rejects a second parentless creation, and `under: None` is otherwise
  rejected (the CLI cannot construct it). The plan emits `Add { what:
  Project(ROOT), under: Desktop }` + header/matrix `AddNested` +
  `ConfigurationChange::AddProject` (idempotent: the parse pre-built the root
  in the aggregate because `to_commands` derives its slot assignments from
  it). The document mirror ignores the root's `AddProject` — the file is flat
  and never names the root.
- The pre-fractal `Desktop` special-cases return, adapted: the
  `LayoutAxis::VERTICAL` measure spec (spacing 0 — the root project is its only
  child), `ResizeAll` measuring `Desktop`, the layout root (`place_root`) at
  `Desktop`, and the hit-test rooted at `Desktop` with a full-window miss
  mapping to `Desktop`.
- `AddProject`'s `placement` is ignored for `under: None` — the root project is
  not hosted by a matrix.
- The root renders exactly like a nested project: header + `PROJECT_PADDING`,
  `PROJECT_HEADER_SPACING` between header and matrix. `SECTION_SPACING`
  disappears.

## 5. Layout: full-size nested scenes, scaled presentation

- The layout algorithm is unchanged at single-project scale; a nested project
  is laid out at its own full size. A project-assigned slot **measures like an
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
- A nested project's presenter node hangs under the *matrix location* of the
  project that hosts its slot, because its layout transform is relative to that
  matrix — the same relation the launcher and instance nodes already have. Only
  the root project hangs under the desktop location. Attaching a nested project
  to the desktop instead drops the hosting matrix's origin, so the scene draws
  the project (and everything below it) offset from the placement the camera,
  the hover outline and hit testing read.
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
  Project depth clamps (no-op; repr `0` makes it the underflow guard).
- **No stored focus context (decided):** the focused project is *derived* from
  the currently focused target by walking the ancestor chain to the nearest
  project in the topology — the Focus Depths stay relative to that project
  and the existing single `focus_depth` field carries the Focus Depth. Entering a
  nested project is a focus change (its target resolves to the project depth
  via the existing nearest-depth walk), not a state change.
- `zoom_in` at Slot depth branches on slot content: launcher → `Instance`,
  project → focus the nested project (entering at its `Project` depth).
- `ZoomOut` from a nested project lands at the parent's Slot depth; the
  focused project re-derives to the parent.
- `project_of_target` answers the root project's id for `Desktop` too — its one
  documented exception: `Desktop` goes *down*, not up (its only child is the
  root project), the exception arm logs, and the derived focused-project walk
  depends on a project being found at the root; the camera walk's
  `expect` stays the repr-0 underflow guard.
- Keyboard navigation: navigating onto a project slot focuses the nested
  `Project` target (the focused project re-derives); cross-project vertical
  overflow becomes sibling-slot navigation through the *parent* slot's sibling
  sequence (column affinity semantics unchanged); a `Project` target becomes a
  navigation origin.
- Camera resolution: project-overview cameras fit the *scaled* rect of a
  nested presentation directly. Deeper target cameras compensate for the
  target placement's composed scale so focused content returns to its local
  pixel scale; no per-level scale reconstruction is needed.
- Indicator: 5 labels, `Project / Row / Slot / Instance / Full Screen` (the
  label array is indexed by repr, so it must match the new declaration order).

## 7. CLI / requests (`massive-terminal` `src/command.rs`,
`src/terminal_request.rs`, `applications/src/instance_environment.rs`)

- `ConfigurationRequest` follows the CLI command vocabulary:
  `AssignLauncher { name, column, row, under }`,
  `AssignProject { name, column, row, under }` (assigning a project creates the
  nested project), distinct `RemoveLauncher` and `RemoveProject` requests,
  `MoveLauncher { direction }`, `PushLauncher { direction }`, and
  `SetStartup { path }`, plus `Resize`/`Undo`/`Redo`. Assignment requests carry
  the `SlotShift` policy, defaulting to `Shift`; the dispatcher translates
  command-shaped requests into slot changes.
- Address paths: `labs/shell` is relative to the invoking instance's project;
  a leading `/` addresses from the root, resolved nearest-match per segment.
- `mt` subcommands: `add project NAME COL ROW [--under PATH]`,
  `add launcher NAME COL ROW [--under PATH]`,
  `remove launcher|project [--name]`, directional `move`/`push` within the
  current matrix, and `startup [PATH]`. Cross-matrix moves are deferred.

## 8. Documentation

- Rewrite `docs/adr/0011-desktop-is-a-fractal-of-projects-in-slots.md` and
  update `CONTEXT.md` to the decisions here: implicit slots and legal empty
  projects, the assign/clear vocabulary with the planning-layer shift policy,
  the flat file with a synthesized `Projects` root (fixed
  [`ProjectId::ROOT`], command-created under the virtual `Desktop` target), the
  `default_panel_size` preview, and the deferral of the depth cut and the
  hit-test rule.

## Risks / watch-outs

- `project_matrix_tracks`/`place_project_matrix_children` zip
  `matrix_launchers` with child sizes; a project slot's size must come
  from `default_panel_size`, or the nested project's own measurement becomes
  the slot size and the scale collapses to 1.
- The presentation scale must be applied once (by the matrix placement), or
  the nested project's presenter doubles it.
- The camera walk's `expect` needs the root to answer as a target (the
  `project_of_target` `Desktop` exception); the hit-test miss returns
  `Desktop`, whose pointer focus must not route into launcher/instance paths.
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
   assign-into-assigned-slot shift and the startup auto-rewrite. Decide whether
   the mirror applies after the live model before those land.
