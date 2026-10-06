# Massive Desktop Interaction Context

This context defines the interaction and presentation language for desktop instance layout and visibility behavior. It exists to keep behavior terms consistent across layout, hit testing, text shaping, and rendering discussions.

## Language

**Desktop configuration**:
The set of projects, their slots (each hosting a launcher or a nested project), and the startup launcher. It persists across sessions and names no root project — the terminal synthesizes one.
_Avoid_: settings, workspace, layout

**Configuration change**:
A change to the desktop configuration: adding the root project, assigning or clearing a slot's content, moving slot content, or setting the startup launcher. Replacing content is a clear plus an assign, never in place. The only kind of change that persists across sessions.
_Avoid_: project change

**Desktop**:
The hierarchy's virtual root target. It is never inserted explicitly, has no presenter, and only parents the root project's target — the one node every hit-test miss maps to. The root project's framing is the outermost camera position.
_Avoid_: desktop depth, root project (the root project hangs *below* it)

**Project**:
A named group that owns a matrix of slots. A project with no slots is empty and still valid; assigning a project name into a slot creates one. Project names may repeat. Exactly one project is the root — synthesized by the terminal with the fixed id `ProjectId::ROOT` and named `Projects`, created by the boot flow's first command (`AddProject { under: None }`) — and every other project is nested in a slot of another project.
_Avoid_: workspace, section, top-level group

**Slot**:
A matrix slot: a position in a project's matrix that hosts either a launcher or a nested project — never both. Slots are implicit: one exists only while a launcher or a nested project is assigned to it, so a cleared slot is empty and a project may have no slots.
_Avoid_: cell, launcher cell, tile, container, occupied

**Matrix placement**:
A slot's (column, row) position in its project's matrix. Placements are unique within a project; adding or moving slot content shifts the placements of the slots it displaces.
_Avoid_: coordinates

**Project focus slot**:
The one slot in a project through which the most recently keyboard-focused target descends. Each project on the focus path remembers only its own immediate slot; nested projects continue the path independently. If no remembered slot is available, focus falls back to the first launcher found depth-first in matrix order, or to the project itself when no launcher exists in its subtree.
_Avoid_: recent instance per project, focus history path

**Intrinsic project scene**:
A project's layout in its own pixel coordinates, including the presentation of its children but excluding presentation scaling assigned by its parent.
_Avoid_: slot size, presented extent

**Shared project scale**:
The uniform presentation scale a parent assigns to all its direct nested projects, across every column. It is determined by the widest intrinsic project scene relative to the preferred default panel width, never enlarges projects, and does not constrain their presented heights.
_Avoid_: per-slot zoom, camera zoom, viewport resize

**Startup launcher**:
The launcher the session boots into. Persisted by address path in the desktop configuration; resolved to a launcher id on load, falling back to the nearest depth-first launcher from the root with a warning when the path no longer resolves.
_Avoid_: startup profile, boot target

**Address path**:
The configuration-level address of a slot's content: slash-separated names resolved segment by segment, picking the nearest match at each level. The invoking instance's project is the implicit base for relative paths; a leading separator addresses from the root, which itself has no name.
_Avoid_: target path, locator

**Launcher id**:
The stable per-session identity of a launcher. Ids are newly created every session and never persist; names are the persistence identity and may repeat, so a name that addresses one launcher resolves to the nearest match.
_Avoid_: profile id, launch profile

**Runtime state**:
The non-persisted desktop state reconstructed every session: running instances, focus, navigation affinity, and window size.
_Avoid_: session state, ephemeral state

**Zoom-out steps**:
The number of framing levels the camera is moved outward from the keyboard-focused target's innermost available framing. Each step moves one level outward through the target's project hierarchy and then its parent projects. The same count is resolved from the new keyboard target after focus changes and is clamped to the levels available on that target's path. An outward step never decreases camera distance, even when its frame bounds are narrower. Clicking a target resets the count to zero; `Cmd+Enter` resets a positive count to zero and defers its usual action until the next press.
_Avoid_: zoom level

**Keyboard-focused target**:
The target that receives keyboard input. Navigation and explicit selection can change it; the camera resolves its current zoom-out steps from this target.
_Avoid_: camera target, hover target

**Hover target**:
The target a click at the pointer would select. For now, descendants remain visible and directly selectable through arbitrary project nesting; clicking a project's header or empty matrix selects that project itself. The target is refreshed when layout or camera changes move content beneath a stationary pointer and is hit-tested again on press. Cursor-motion events are sent only in response to physical cursor movement.
_Avoid_: keyboard-focused target, camera focus

**Hover outline**:
The rectangle identifying the pointer's click destination. It is absent while pointer feedback is suppressed; keyboard focus has a separate indication and is never substituted as the hover target.
_Avoid_: focus rectangle, keyboard-focus indicator

**Pointer event target**:
The target receiving pointer events. It changes on physical pointer movement or a click, not merely because camera or layout changes move content beneath a stationary pointer.
_Avoid_: hover target

**Keyboard-focused project**:
The project containing the keyboard-focused target. Project-relative paths and commands use it as their base, regardless of zoom-out steps.
_Avoid_: current project, camera project

**Navigate**:
Directional movement of keyboard focus one step from the current target, driven by an arrow key. At an outward zoom position, navigation selects a concrete slot in the destination row or matrix, using the project's focus slot and the launcher's instance anchor to resolve the concrete target.
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

**Base instance**:
An instance a launcher starts with the launcher's configured parameters — through the launcher's start action, or with `Cmd+T` / `Cmd+Enter` while one of the launcher's instances has keyboard focus. All base instances of a launcher follow the launcher's Full Screen Mode together; the assistant instance is the other kind.
_Avoid_: default instance, primary instance

**Assistant instance**:
An instance created through the assistant entry point: `Shift+Cmd+T` / `Shift+Cmd+Enter` starts one without the launcher's configured parameters. An assistant owns a temporary Full Screen Mode of its own, unaffected by its launcher's; unlike the launcher's, it never persists.
_Avoid_: aux instance, sidecar, secondary instance

**Instance**:
A single running application session owned by a launcher. Multiple instances of the same launcher can coexist, are presented by the visor, and appear or disappear dynamically as the user opens or closes them.
_Avoid_: session, tab, process

**Full Screen Mode**:
An instance presentation state that scales the instance's content toward the window instead of its regular panel scale. It exists per launcher — one value shared by all of its base instances, persisted with the desktop configuration — and per assistant instance, where it is temporary. The `Ctrl+Cmd+F` View-menu action first enters native window fullscreen whenever the window is windowed, regardless of focus. Once the native window is fullscreen, the action toggles Full Screen Mode only when zoom-out steps are zero and keyboard focus resolves to an instance: a base instance toggles its launcher's mode, an assistant toggles its own. Otherwise that action exits native window fullscreen. `Cmd+Enter` resets a positive zoom-out count before its usual action; at zero it starts an instance only from a launcher. `Cmd+T` starts an instance from a launcher or an existing instance. Starting an instance focuses it while preserving the current zoom-out count, clamped to the new focus path. Focus, navigation, and resize never change Full Screen Mode. A focused instance presented at its Full Screen scale resolves as pixel-perfect at that scale automatically. Full Screen Mode affects only the instance's content scaling — never the launcher's presentation or the visor layout.
_Avoid_: fullscreen flag, fullscreen presentation, zoom level, full screen launcher

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
