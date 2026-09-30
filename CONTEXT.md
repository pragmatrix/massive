# Massive Desktop Interaction Context

This context defines the interaction and presentation language for desktop instance layout and visibility behavior. It exists to keep behavior terms consistent across layout, hit testing, text shaping, and rendering discussions.

## Language

**Desktop configuration**:
The set of projects, launchers, their slot placements, and the startup launcher. It persists across sessions.
_Avoid_: settings, workspace, layout

**Configuration change**:
A change to the desktop configuration: adding or removing a slot or its content, moving slot content, or setting the startup launcher. The only kind of change that persists across sessions.
_Avoid_: project change

**Project**:
A named group that owns a matrix of slots. A project exists only while it has at least one slot; removing a launcher removes the slot and, with it, the project. Project names may repeat. Exactly one project is the root; every other project is nested in a slot of another project.
_Avoid_: workspace, section, top-level group

**Slot**:
A cell in a project's matrix that hosts either a launcher or a nested project — never both. Slots present one slot level deep when their project is not the focused project: slots render without their contents' contents and nothing deeper is interactive. Presenters are created for non-empty slots.
_Avoid_: launcher cell, tile, container

**Matrix placement**:
A slot's (column, row) position in its project's matrix. Placements are unique within a project; adding or moving slot content shifts the placements of the slots it displaces.
_Avoid_: coordinates

**Startup launcher**:
The launcher the session boots into. Persisted by address path in the desktop configuration; resolved to a launcher id on load, falling back to the root project's first launcher with a warning when the path no longer resolves.
_Avoid_: startup profile, boot target

**Address path**:
The configuration-level address of a slot's content: slash-separated names, resolved segment by segment, picking the nearest match at each level. The invoking instance's project is the implicit base for relative paths; a leading separator addresses from the root.
_Avoid_: target path, locator

**Launcher id**:
The stable per-session identity of a launcher. Ids are newly created every session and never persist; names are the persistence identity and may repeat, so a name that addresses one launcher resolves to the nearest match.
_Avoid_: profile id, launch profile

**Runtime state**:
The non-persisted desktop state reconstructed every session: running instances, focus, navigation affinity, and window size.
_Avoid_: session state, ephemeral state

**User state**:
The system-level interaction mode that decides what the camera follows. Either `Focused` or `Overview`.
_Avoid_: view mode, camera mode flag

**Focused**:
The default user state where the camera follows the keyboard-focused target.
_Avoid_: normal mode, zoomed-in

**Overview**:
A user state where the camera detaches from focus and follows a separate overview target while keyboard focus stays put. `Ctrl+Down` enters or climbs it one hierarchy level per press; `Ctrl+Up` zooms back in one level; any non-navigation command returns to `Focused`.
_Avoid_: zoomed out (the `ZoomOut` command name is retained, but the state is "overview"), bird's-eye

**Overview target**:
The hierarchy target the camera follows while in `Overview`. Climbs toward the root on each `ZoomOut` and pans among same-level siblings on `Navigate`.
_Avoid_: camera anchor, zoom target

**Focused project**:
The project whose depth ladder the overview depth resolves against. Changed by zooming through a project slot: entering a nested project makes it the focused project; zooming out past its Project depth returns to the parent slot and makes the parent the focused project.
_Avoid_: current project (ambiguous with the invocation base), active project

**Focus depth**:
One rung of the overview's depth ladder — Project, Project Row, Slot, Instance, Full Screen — resolved relative to the focused project. The ladder reads outermost first, so a rung's position counts the zoom-ins from the project level. There is no rung above the root project's Project depth; ZoomOut at that floor is a no-op.
_Avoid_: zoom level (the Zoom commands are retained), Desktop depth (the root is the floor), Row (the rung names the owning project)

**Navigate**:
Directional movement of keyboard focus (or the overview target) one step from the current position, driven by an arrow key.
_Avoid_: move, arrow

**Navigate to target**:
An explicit keyboard-focus change to a named target (or to nothing, which only removes focus and has no camera effect). It is the deferred outcome of a pointer click or a window focus change, applied as a command rather than mutated inline.
_Avoid_: set focus, click focus

**Focus suggestion**:
The event router's proposal that keyboard focus should change, surfaced from input processing instead of being applied directly. It is lowered into a navigate-to-target command, which owns the actual focus change, focus-driven relayout, and anchor sync.
_Avoid_: pending focus, focus request

**Launcher**:
A configured entry that a slot may host; hosts one or more running instances and owns a launcher mode. A launcher holds a dynamic set of instances that can be added (or removed) at runtime, so its instance count is not fixed by configuration even when it starts from a single command.
_Avoid_: profile, cell

**Instance**:
A single running application session owned by a launcher. Multiple instances of the same launcher can coexist, are presented by the visor, and appear or disappear dynamically as the user opens or closes them.
_Avoid_: session, tab, process

**Close request**:
A window-lifecycle signal that asks the application to end. It is delivered to every live instance regardless of keyboard focus because closing the application is not a focus-targeted interaction.
_Avoid_: close click, focused close event

**Graceful shutdown**:
The bounded application lifecycle from a close request through instance termination, final submission processing, renderer teardown, and native-window release.
_Avoid_: window close, process exit

**Shutdown deadline**:
The single monotonic deadline that bounds graceful shutdown. Once it expires, the desktop ends immediately and the shell terminates the process without waiting for unfinished instances or submissions.
_Avoid_: per-instance timeout, forced-shutdown state

**Placement visibility**:
A semantic flag on placement that states whether an instance should be interactable and visually present in the current layout state.
_Avoid_: hidden by alpha, render-only visibility

**Collapsed visor**:
A launcher state where only the center visor instance remains visible while non-center instances transition out.
_Avoid_: minimized stack, folded carousel

**Center visor instance**:
The visor focus anchor the visor centers on and that stays visible during collapse: the most recently focused instance while no mouse button was pressed. The visor centers on this anchor independent of the live keyboard focus.
_Avoid_: active card, selected panel, currently focused instance

**Non-center visor instance**:
Any visor instance that is not the center instance and is transitioned to invisible in collapsed state.
_Avoid_: background card, side panel

**Structural animation**:
The shared layout transition animation used for placement changes, including transform and visibility alpha transitions.
_Avoid_: ad-hoc tween, per-feature animation

**Visibility alpha**:
An animation channel that drives fade-in and fade-out based on placement visibility and composes with view alpha.
_Avoid_: opacity hack, visual-only alpha

**Hidden depth baseline**:
The z position used when an instance becomes invisible so hidden visor panels return to baseline depth while fading.
_Avoid_: parked z, offscreen depth

**Visibility-gated hit testing**:
The rule that invisible placements are excluded from hit-testing immediately, independent of in-flight fade animation.
_Avoid_: alpha-threshold hit test, delayed interaction disable

**Shaping engine**:
The selectable implementation that shapes attributed text into glyph runs for rendering. Either `Parley` (default) or `CosmicText`, both behind one shaping contract producing the same neutral glyph data.
_Avoid_: font system, shaper backend, text layout engine

## Font identity language

**Loaded face**:
A font face registered because the application explicitly passed its bytes to `FontManager::load_font`. The client-directed way a face enters the identity world.
_Avoid_: registered face

**Resolved face**:
A font face the shaper selected while shaping (typically as a fallback for a codepoint the requested family lacks) that was not loaded beforehand. It is registered on first use through the face authority and published immediately.
_Avoid_: interned face, interning (collides with string interning), lazy intern, adopted face

**Face authority**:
The single issuer of `FaceId`s — the canonical shaping engine behind the `FontManager` mutex. All loaded and resolved faces enter the identity world through it.

**Candidate pool**:
Faces available to the shaper for implicit selection (system fonts when the manager is created with `system()`) that are not in the identity world. A candidate pool face exists only for selection; it joins the published world only once actually resolved.
_Avoid_: system font db, fallback fonts (as a synonym for the pool)

**Published registry**:
The immutable snapshot mapping every known `FaceId` (loaded and resolved faces) to font data and metrics, read lock-free by the renderer and by a shaping context's own resolution.
_Avoid_: font registry (ambiguous with the candidate pool)

**Registry sync**:
The per-shape step where a scratch compares the published registry's face count against its last-seen count and loads faces it has not seen yet. Keeps a context's scratch aligned with the identity world without locking; it runs before every shape, so a loaded face is visible to the next shape rather than the next batch.
_Avoid_: epoch sync, epoch-pull, seed

**Shaping session**:
One shaping batch over a context: the context plus its registry snapshot, opened by
`task_context::shaper` and dropped before the frame's output is submitted.
_Avoid_: session, shaper session, shaper handle

**Bare manager**:
A font manager with no fonts and no fallback candidates, so selection can only reach fonts the application loaded itself.
_Avoid_: empty manager, registry-only collection

**Font policy**:
The pair of a shaping engine and whether system fonts are available for selection. Named by the
client and passed to `shell::run`, which builds the application task's font manager from it and
installs the task's shaping context in the same step. A face is only meaningful within the manager
that issued it, so the manager a task shapes with cannot change after that.
_Avoid_: font settings, font config

**Shaping context**:
A task's shaping owner: it *is* the shaping session, holding the exclusive shaping scratch over the shared font manager's face authority and published registry. Every task context has exactly one; on the ambient path it is lent out by the `task_context::shaper` guard.

**Task context**:
The contexts installed for one Tokio task: its change queue, animation coordinator, movement runtime, and shaping context.

**Change queue**:
The changes one task submits together, in the order their writers produced them. Each UI
task owns exactly one queue; its change type (`SceneChange` for the application task's
render queue, `InstanceChange` for an instance's submission queue) is fixed at install
time, and the frame drains the queue into a submission at animation-cycle end (ADR 0008).
_Avoid_: change stream

**Change collector**:
The lock-guarded accumulation a change queue is stored in: an ordered set of changes of
exactly one type. The change type is fixed when the collector is created, so a collector
never has to check what it receives; draining takes the accumulated changes out in order.
Every change queue is backed by exactly one collector.
_Avoid_: change buffer, change list

**Change sink**:
The write-only view of a change queue that handles hold. It accepts scene changes and
retypes them into the queue's own change type, so a handle never knows the queue's change
kind. Erasing the sink is what lets one queue per task serve every handle.
_Avoid_: change stream, sender, channel

**Submit**:
Create an object handle and add it to the task's change queue: the handle publishes
its create into that queue, and its later updates and deletion follow the same path.
Because handles hold the erased sink, their changes land in the same FIFO as the task's
own changes.
_Avoid_: connect, enter

**Mounting**:
Wiring a movement's value and apply-animations callback into a task's movement runtime, done once at presenter construction, before any frame exists. Mounting only enqueues the movement's actions; it does not animate or read the animation clock.

**Exclusive animation-cycle lease**:
The live `Frame` value that owns one animation cycle and prevents another cycle from using the same mutable animation contexts concurrently.

**Shaping scratch**:
Per-task font and layout state owned by a shaping context, distinct from the shared font registry. It outlives a batch, so its fallback-resolution caches survive across frames.
