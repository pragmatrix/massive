# Massive Desktop Interaction Context

This context defines the interaction and presentation language for desktop instance layout and visibility behavior. It exists to keep behavior terms consistent across layout, hit testing, text shaping, and rendering discussions.

## Language

**Desktop configuration**:
The set of projects, their assigned slots (each hosting a launcher or a nested project), and the startup launcher. It persists across sessions and names no root project — the terminal synthesizes one.
_Avoid_: settings, workspace, layout

**Configuration change**:
A change to the desktop configuration: adding the root project, assigning content to a slot or clearing it, moving slot content, or setting the startup launcher. Replacing content is a clear plus an assign, never in place. The only kind of change that persists across sessions.
_Avoid_: project change

**Configuration command**:
An operation a user asks for on the desktop configuration, such as assigning, clearing, moving, swapping, or pushing slot content, adding content next to a slot, grouping, ungrouping, or setting the startup launcher. It names its slots by ids and slot positions. The keyboard, the CLI, and the boot replay each turn what they mean into one, and the desktop system plans it into configuration changes, together with where the focus goes afterwards (ADR 0024). A `ConfigurationRequest` is not one: it is the CLI's message, which names content by names and target paths and becomes a command.
_Avoid_: configuration request

**Desktop**:
The hierarchy's virtual root target. It is never inserted explicitly and only parents the root project's target — the one node every hit-test miss maps to. The root project's framing is the outermost camera position.
_Avoid_: desktop depth, root project (the root project hangs *below* it)

**Project**:
A named group that owns a matrix of slots. A project without assigned slots is empty and still valid; assigning a project name into a slot creates one. Project names may repeat. Exactly one project is the root — synthesized by the terminal with the fixed id `ProjectId::ROOT` and named `Projects`, created by the boot flow's first command (`AddProject { under: None }`) — and every other project is nested in a slot of another project.
_Avoid_: workspace, section, top-level group

**Slot**:
A position in a project's matrix, identified by its slot position. Every position is a slot. An assigned slot hosts either a launcher or a nested project, never both; an unassigned slot is empty. Clearing a slot makes it unassigned. Only assigned slots are stored, so a project whose slots are all unassigned stores none.
_Avoid_: cell, launcher cell, tile, container, occupied, empty placement

**Assigned slot**:
A slot that hosts a launcher or a nested project. Assigning content to an assigned slot replaces it, which is a clear plus an assign.
_Avoid_: occupied slot, filled slot

**Unassigned slot**:
A slot that hosts nothing. It is not stored, but it can still be named, for example by a selection or as the destination of a move.
_Avoid_: empty placement, gap, free placement

**Slot position**:
A slot's column and row in its project's matrix, the column counted first. Positions are unique within a project; adding or moving slot content shifts the positions of the assigned slots it displaces.
_Avoid_: matrix placement, placement, coordinates, index

**Matrix interval**:
The stretch a column or a row of a project's matrix covers along its axis. Every column interval is one slot extent wide, and a row interval is as tall as the tallest slot in it. Intervals are separated by a fixed spacing.
_Avoid_: track, lane, band

**Selection**:
A rectangle of slots in one project's matrix that contains the keyboard-focused slot. It is two offsets of columns and rows from that slot's position, one towards its top-left slot and one towards its bottom-right slot, and the zero offsets select only the focused slot. The rectangle includes unassigned slots, and the operations that act on a selection act on the assigned slots inside it. Esc or Cmd+Esc, and any focus, zoom, or configuration change, collapses it to the zero offsets.
_Avoid_: anchor, far corner, multi-selection

**Extended selection**:
A selection whose offsets are not both zero, so it reaches beyond the focused slot, even when only unassigned slots lie beyond. Shift+Cmd+arrow extends a selection, and Cmd+A extends it over all assigned slots of its project.
_Avoid_: widened selection, multi-selection

**Slot extent**:
The fixed size every slot's content fits into: one instance extent, the default panel with its title bar. Every column of a matrix is as wide as it, no row is taller, and content smaller than its slot sits centered in it.
_Avoid_: cell, slot size, default size

**Project focus slot**:
The one slot in a project through which the most recently keyboard-focused target descends. Each project on the focus path remembers only its own immediate slot; nested projects continue the path independently. If no remembered slot is available, focus falls back to the first launcher found depth-first in matrix order, or to the project itself when no launcher exists in its subtree. Zooming in on a project restores its focus slot one level deep; entering it follows the focus slots down to a leaf.
_Avoid_: recent instance per project, focus history path

**Intrinsic project scene**:
A project's layout in its own pixel coordinates, including the presentation of its children but excluding presentation scaling assigned by its parent.
_Avoid_: slot size, presented extent

**Shared project scale**:
The uniform presentation scale a parent assigns to all its direct nested projects, across every column: the largest scale that fits both the widest and the tallest intrinsic project scene into one slot extent. It never enlarges projects.
_Avoid_: per-slot zoom, camera zoom, viewport resize

**Startup launcher**:
The launcher the session boots into. Persisted by target path in the desktop configuration; resolved to a launcher id on load, falling back to the nearest depth-first launcher from the root with a warning when the path no longer resolves.
_Avoid_: startup profile, boot target

**Target path**:
The configuration-level path to a slot's content: slash-separated names resolved segment by segment, picking the nearest match at each level. The invoking instance's project is the implicit base for relative paths; a leading separator starts from the root, which itself has no name.
_Avoid_: address path, address, locator

**Launcher id**:
The stable per-session identity of a launcher. Ids are newly created every session and never persist; names are the persistence identity and may repeat, so a name that several launchers share resolves to the nearest match.
_Avoid_: profile id, launch profile

**Runtime state**:
The non-persisted desktop state reconstructed every session: running instances, focus, navigation affinity, and window size.
_Avoid_: session state, ephemeral state

**Instance title bar**:
The bar the desktop draws above an instance's panel, outside the instance's content grid, so the content's size is unaffected. It is one left-aligned string: the primary view's title (for the terminal, its working directory), a separator, then the launcher's name (marked as an assistant for an assistant instance); the name alone while the title is empty. It exists for every instance with a primary view, has a fixed height at regular presentation scale even in Full Screen Mode, and is part of the instance's extent for layout. It follows the instance's placement visibility and structural animation, and a click on it selects the instance. It is the instance-level counterpart of a project header.
_Avoid_: status panel, status bar, instance header

**Part**:
A target that belongs to a project or an instance and is laid out on its own: a project's header and matrix, an instance's title bar and views. A part selects the target it belongs to, except a view, which holds the keyboard focus itself.
_Avoid_: sub-target, component

**Zoom level**:
What the camera frames relative to the keyboard-focused target, within its zoom project: `Project`, `Row`, `Slot` or `Focus`. `Focus` frames the target itself — an instance's panel together with its title bar, or for any other target the same rect as its slot. Navigation keeps the zoom level. Zooming out of a nested project's outermost row focuses that project as a slot of its parent; zooming in on a project target focuses its project focus slot at `Row`. Clicking a target, starting an instance, and `Cmd+Enter` set `Focus`; `Cmd+Enter` or `Enter` on a project target first enters it down to its focused leaf. A target is fully in when its zoom level is `Focus`. The indicator shows what is framed — `Focus` reads `Instance` on an instance and `Slot` otherwise — with the zoom project's nesting depth, root = 0, e.g. `Row 1`.
_Avoid_: zoom depth, zoom-out steps, focus depth, framing level, innermost

**Zoom project**:
The project whose matrix holds the keyboard-focused target's slot. A project target is a slot of its parent; the root project, which no matrix hosts, is its own zoom project.
_Avoid_: framed project, camera project

**Keyboard-focused target**:
The target that receives keyboard input: an instance, a launcher, or a project focused as a slot. Navigation, zooming across a project boundary, and explicit selection can change it; the camera frames it at the zoom level.
_Avoid_: camera target, hover target

**Hover target**:
The target a click at the pointer would select. For now, descendants remain visible and directly selectable through arbitrary project nesting; clicking a project's header or empty matrix selects that project itself. The target is refreshed when layout or camera changes move content beneath a stationary pointer and is hit-tested again on press. Cursor-motion events are sent only in response to physical cursor movement.
_Avoid_: keyboard-focused target, camera focus

**Hover outline**:
The rectangle identifying the pointer's click destination while the pointer drives the interaction. While the input focus is at a desktop level it marks the keyboard-focused target instead; once the input focus reaches the application it is absent.
_Avoid_: focus rectangle, keyboard-focus indicator

**Pointer event target**:
The target receiving pointer events. It changes on physical pointer movement or a click, not merely because camera or layout changes move content beneath a stationary pointer.
_Avoid_: hover target

**Input focus**:
The deepest target on the keyboard focus path that consumed the most recent key press, so it never lies beyond the keyboard-focused target. It is absent while the pointer drives. While it is a desktop-level target, the user is still working with the desktop and the hover outline marks the keyboard-focused target; once it is the application, the user is working inside the application and the hover outline is gone.
_Avoid_: key scope, last receiver, focus range

**Keyboard-focused project**:
The project containing the keyboard-focused target. Project-relative paths and commands use it as their base, regardless of zoom level.
_Avoid_: current project, camera project

**Navigate**:
Directional movement of keyboard focus one step from the current target, driven by an arrow key. It depends only on the keyboard-focused target, never on the zoom level. When the project's matrix has no neighbor in that direction, the search escapes to the slot hosting the project in its parent's matrix and retries from there, up to the root; with no neighbor at any level the key does nothing. Landing on a project slot enters the project through its focus slots down to a leaf, so moving back restores the focus; a project with no remembered focus slot is focused as a slot itself. Landing on a launcher slot focuses its instance anchor, else its directional edge instance.
_Avoid_: move, arrow

**Escape**:
Moving the keyboard focus one level out with Esc or Cmd+Esc, as **Navigate** escapes to the slot hosting a project. A launcher or a nested project hands the focus to the project whose matrix holds it; in the root project, the camera frames the whole root. Plain Esc leaves a focused instance to its application, while Cmd+Esc escapes the instance to its launcher. An extended selection collapses first.
_Avoid_: leave, climb, exit

**Navigate to target**:
An explicit keyboard-focus change to a named target (or to nothing, which only removes focus and has no camera effect). It is the deferred outcome of a pointer click or a window focus change, applied as a command rather than mutated inline.
_Avoid_: set focus, click focus

**Focus suggestion**:
A proposal from input processing that keyboard focus should change, instead of applying it directly: a pointer press reported by the event router, or the target a window focus change restores, decided by the keyboard focus beside the router (ADR 0021). It is lowered into a navigate-to-target command, which owns the actual focus change, focus-driven relayout, and anchor sync.
_Avoid_: pending focus, focus request

**Launcher**:
A configured entry that a slot may host; hosts one or more running instances and owns a launcher mode. A launcher holds a dynamic set of instances that can be added (or removed) at runtime, so its instance count is not fixed by configuration even when it starts from a single command. `Cmd+N` adds a new, empty launcher right of the focused slot, shifting the slots to its right, and focuses it.
_Avoid_: profile, cell

**Primary instance**:
An instance a launcher starts with the launcher's configured parameters — through the launcher's start action, or with `Cmd+T` / `Cmd+Enter` while one of the launcher's instances has keyboard focus. All primary instances of a launcher follow the launcher's Full Screen Mode together; the assistant instance is the other kind. In code it is `InstanceKind::Primary`. The name is provisional and is not related to an instance's primary view, which is the view that gives the instance its title.
_Avoid_: base instance, default instance

**Assistant instance**:
An instance created through the assistant entry point: `Shift+Cmd+T` / `Shift+Cmd+Enter`, or a Shift-click on a launcher, starts one without the launcher's configured parameters. An assistant owns a temporary Full Screen Mode of its own, unaffected by its launcher's; unlike the launcher's, it never persists.
_Avoid_: aux instance, sidecar, secondary instance

**Application**:
The program an instance runs, such as the Markdown viewer. It runs in its own task and draws into the instance's views. The desktop reaches it only through the instance manager, by sending view events. It cannot report back whether it used an event. A key that no desktop level consumed is forwarded to the application.
_Avoid_: app

**Instance**:
A single running application session owned by a launcher. Multiple instances of the same launcher can coexist, are presented by the visor, and appear or disappear dynamically as the user opens or closes them.
_Avoid_: session, tab, process

**Full Screen Mode**:
An instance presentation state that scales the instance's content toward the window instead of its regular panel scale. It exists per launcher — one value shared by all of its primary instances, persisted with the desktop configuration — and per assistant instance, where it is temporary. The `Ctrl+Cmd+F` View-menu action first enters native window fullscreen whenever the window is windowed, regardless of focus. Once the native window is fullscreen, the action toggles Full Screen Mode only when keyboard focus is fully in and resolves to an instance: a primary instance toggles its launcher's mode, an assistant toggles its own. Otherwise that action exits native window fullscreen. `Cmd+Enter` zooms to `Focus` before its usual action; when fully in, it starts an instance only from a launcher. `Cmd+T` starts an instance from a launcher or an existing instance. Starting an instance focuses it at the `Focus` zoom level. Focus, navigation, and resize never change Full Screen Mode. A focused instance presented at its Full Screen scale resolves as pixel-perfect at that scale automatically. Full Screen Mode affects only the instance's content scaling — never the launcher's presentation or the visor layout.
_Avoid_: fullscreen flag, fullscreen presentation, full screen launcher

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

**Exceptional behavior**:
A desktop behavior that deviates from the plain outcome of what the user asked for or from the obvious default, and that could reasonably have been decided differently. Refusing to remove slot content while instances run below it is one example. An outcome the model makes impossible, such as removing the root project, is an invariant, not an exceptional behavior.
_Avoid_: special case, edge case, quirk

**Desktop policy**:
The one place that holds the desktop's choices that could reasonably have been made differently. In code it is the desktop's `policy` module. Its rules decide every exceptional behavior (ADR 0029). For a command, a rule allows it, refuses it with a reason, or redirects it to another command. For a lifecycle default or a route, a rule chooses the outcome. Its tuning values are the sizes, durations, thresholds, easing curves, and colors that were picked by taste and usability experience.
_Avoid_: policy (alone in prose, it collides with font policy), rules, guards

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
