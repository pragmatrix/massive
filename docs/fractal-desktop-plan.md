# Fractal desktop — implementation plan

Derived from the grilling session (glossary in `CONTEXT.md`, decision record in
`docs/adr/0011-desktop-is-a-fractal-of-projects-in-slots.md`). Ordered so every
step compiles.

## 1. Model: slots in the runtime tree

The runtime tree keeps content targets (`Project`, `Launcher`, `Instance`,
`View`) but loses the implicit two-level assumption: a `ProjectMatrix`'s
children become a slot-disciplined sequence. Four panics in
`topology.rs` (`launcher_of_instance`, `project_of_launcher`,
`matrix_launchers`) type-enforce launcher-only matrix children and must
generalize: the matrix returns ordered **slot entries**, each classifying its
content as `Launcher(LaunchProfileId)` or `Project(ProjectId)`. Call sites
(layout, navigation, removal planning, boot) consume slot entries instead of
assuming the cell kind from the parent.

Key sub-points that scan of the current code surfaced:

- **Elaboration (decided): slot entries, not a `Slot` topology node.** No
  `SlotId` namespace, no third id alongside `ProjectId`/`LaunchProfileId`. The
  slot is *(matrix placement → content)*, addressable by
  `(parent ProjectId, MatrixPlacement)` — the same key
  `RemoveSlotShiftingPolicy` shifts by today. What a slot needs beyond the
  placement is a *presenter target it can point at*: hover outline and
  click-to-zoom currently route only `Launcher | Instance | View` targets
  (`presentation.rs`), so the click/hover target for a project-occupied slot
  is the nested `Project` target itself — which already exists as
  `DesktopTarget::Project(_)` and already has camera-resolution entries
  (`Project*` → project depth). Launcher-occupied slots keep targeting the
  launcher. This keeps `DesktopTarget` unchanged except for dropping
  `Desktop`.
- **Shifts stay content-agnostic per cell** (decided: project slots shift
  like launchers), which is exactly the per-cell shape the existing
  `RemoveSlotShiftingPolicy` + `shifted_left_launchers` code already has —
  they generalize from launchers to slot entries by placement, not by needing
  a cell id.
- **`matrix_launchers`' zip-with-children order contract**: the layout
  algorithm zips `matrix_launchers(project)` against the matrix's children
  list; the classification of slot entries must therefore preserve the same
  child-order contract or the zip becomes a placement-keyed join.
- **Aggregates**: `Aggregates` keeps flat
  `projects`/`launchers`/`instances` maps (unchanged); the matrix content
  classification is derived, not stored — a cell's content kind is
  discoverable from the configuration aggregate (`Slot { placement, content
  }`, step 2) or from the topology parent kind itself (`DesktopTarget::
  Project(id)` under a matrix *is* the slot content). No extra runtime state
  is created for slots.
- **Presenters**: no separate `SlotPresenter` kind at first — a
  launcher-occupied slot is presented by the existing `LauncherPresenter`
  (its outline/visors already render the cell), and a project-occupied slot
  is presented by the nested `ProjectPresenter` scaled down (step 4). A
  placeholder for empty cells follows the same pattern as today's launchers.
- Topology rules in `topology.rs`: a slot's content is `Project` or `Launcher`,
  mutually exclusive; root = exactly one project.
- Presenters for newly occupied cells follow the existing presenter kinds (see
  above); no slot-specific runtime state is created.

## 2. Configuration aggregate and change vocabulary

- `configuration.rs`: `Project { slots: Vec<Slot> }`, `Slot { placement,
  content: Launcher | NestedProject(name, …) }`; ≥1-slot invariant.
- `ConfigurationChange`: `AddSlot { parent, placement, content }`,
  `RemoveSlot { target }`, `MoveSlot { source, dest parent, placement }`
  (shift events on both matrices), `SetStartup { target }`.
- Cascading removal: last slot removed ⇒ project removed (existing
  `reject_emptying_removal` semantics generalize).
- Startup pointer auto-rewritten on rename/move while the target id exists;
  unresolved at load ⇒ nearest-match then root's first launcher + warning.

## 3. KDL document

- Grammar (unchanged `launcher`, new nested `project`):
  ```kdl
  project "root" {
      launcher "dev" column=0 row=0 mode=band { spawn "…" }
      project "labs" column=1 row=0 {
          launcher "shell" column=0 row=0
      }
  }
  startup "/root/dev"
  ```
- Persistence tags (`NodeTags`) extend to nested projects; apply change
  surgically as today (ADR 0006/0010 preserved); nearest-match resolution per
  segment for `ConfigurationRequest` targets and `startup`.

## 4. Layout: full-size nested scenes, scaled presentation

- Layout algorithm unchanged at single-project scale; a nested project gets
  laid out at its own full size, its slot presenter applies a uniform
  presentation scale (slot rect / project size) — *not* via `PixelCamera`
  (ADR 0004: camera dolly-only).
- Presentation scale rides the layout transform: `set_layout` already carries
  a full `SizedTransform`, so the slot's presenter applies scale in the same
  channel it already uses for position/size — no new transform channel, and
  hover rects / hit-test transforms scale with it for free.
- Depth cut is a **layout visibility rule**, decided by the layout placement:
  the placer emits `Placement::visible = false` for a slot's *contents* when
  their project is not the focused project — launcher slots stay visible and
  clickable, everything below them (instances and nested slot children) is
  cut. `absolute_placement` ANDs the flag down the ancestor path
  (`desktop_system/layout_state.rs`), so one false parent masks a whole
  nested project; hit testing is already visibility-gated, so render and
  interaction share the one rule (ADR 0011).
- Pruning mechanics, in the existing idiom: the visibility flag drives a
  **bounded fade** on the scene's inherited `Location` alpha (hidden
  launcher/instance treatment: alpha → 0 plus z pulled to the hidden-depth
  baseline). The movement runs only while an animation is pending
  (`animation/movement_runtime.rs`: `ending_time` cleared at the end, then
  the movement is at rest) — after the fade, a pruned subtree costs only the
  renderer's per-visual `alpha == 0.0 → continue` early-out
  (`renderer/src/renderer.rs`), nothing animates, nothing draws. Scene nodes
  and presenters stay alive, so re-focusing a project just flips visibility
  and fades in.
- Deferred: true skipping (drop `Measure` targets under pruned projects in
  the `ChangeSurface` conversion) only removes the residual iteration cost —
  keep for profiling-driven optimization; see Open issues.

## 5. Focus depth and navigation

- `FocusDepth`: `{Project, ProjectRow, Slot, Instance, InstanceFullScreen}` —
  read outermost first (rename `Launcher`→`Slot`, `Row`→`ProjectRow`, drop
  `Desktop`), so the `u8` discriminant counts zoom-ins from the project level;
  `ZoomOut` at root Project floor clamps (no-op; repr `0` makes the floor the
  underflow guard).
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
- Camera resolution: the camera fits the *scaled* rect of a nested
  presentation directly, like any other content — no composed per-level
  camera math. Focused-project cameras per existing per-depth code.
- Indicator: 5 labels, `Project / Project Row / Slot / Instance / Full
  Screen`.

## 6. Interaction

- Hit testing: one-slot-deep rule; slot click = zoom-in gesture; nothing
  deeper hit-testable; focused project unchanged.

## 7. CLI / requests (`massive-terminal` `src/command.rs`,
`src/terminal_request.rs`, `applications/src/instance_environment.rs`)

- Address paths; base = invoking instance's project; leading `/` = root.
- `mt` subcommands: `add project NAME column row [under PATH]`,
  `add launcher NAME column row [under PATH | relative]`,
  `remove launcher|project --name` (nearest-match from current project),
  `push/move` unchanged plus cross-matrix destination, `startup [PATH]`.

## 8. Migration

- First `project` in an old file becomes the root; every other top-level
  project's subtree is nested into a synthesized launcher-named slot (slot
  name = old project name) appended to the root matrix. Old `startup "name"`
  resolves against the migrated tree, else fallback.

## 9. Cleanup

- Remove `Desktop` depth from indicator labels, `zoom_navigation.rs`'s
  desktop camera, `focus_depth_indicator.rs`, `focus_input.rs` floor clamps;
  update `command.rs` docs and any `DesktopTarget::Desktop` matches.

## Open issues

Deferred until the model work (steps 1–6) is in — they touch configuration,
persistence, and boot, which the model change does not depend on.

1. **Parser drops nested project nodes** (`persistence/document.rs`:
   "Ignoring unknown node" in project) — must teach the parser the new node
   before any migrated file round-trips correctly. First persistence change.
2. **Pruning "true skip"** — dropping `Measure` targets under pruned
   (non-focused) projects in the `ChangeSurface` conversion removes the
   residual per-visual iteration cost; only worth it if profiles show hidden
   subtrees mattering. Profiling-driven.
2. **Shift emits per-content moves** — whether the KDL mirror keeps
   `MoveLauncher`-per-shifted-entry or gains `MoveSlot`; decide when step 2
   lands.
3. **Startup auto-rewrite inside transactions** — the document mirror applies
   changes before the live model with no rollback; a slot cascade failing
   mid-transaction is worse under nesting. Doc mirror ordering may need
   fixing before `MoveSlot`/cascades ship.
4. **Undo/Redo** — pending (`todo!()` in `command_dispatch.rs`); out of scope
   until after the model works.
5. **Boot/migration semantics** — flat boot command stream, migration as
   parse-time view vs. write-back (ADR 0006 comment preservation), `Setup`
   transaction effects during the first frame with nested projects booting.
6. **`Undo/Redo` request arms** — `todo!()`; same deferral as 4.
7. **Address-path resolution in requests** (step 7) — instance→project walk
   and base resolution; deferred with the CLI work.