# Massive Desktop Interaction Context

This context defines the interaction and presentation language for desktop instance layout and visibility behavior. It exists to keep behavior terms consistent across layout, hit testing, text shaping, and rendering discussions.

## Language

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
A configured entry in the project matrix that hosts one or more running instances. A launcher holds a dynamic set of instances that can be added (or removed) at runtime, so its instance count is not fixed by configuration even when it starts from a single command.
_Avoid_: profile, slot, tile

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
_Avoid_: minted face, minted font, interned face, interning (collides with string interning), lazy intern, adopted face

**Face authority**:
The single issuer of `FaceId`s — the canonical shaping engine behind the `FontManager` mutex. All loaded and resolved faces enter the identity world through it.
_Avoid_: mint authority

**Candidate pool**:
Faces available to the shaper for implicit selection (system fonts when the manager is created with `system()`) that are not in the identity world. A candidate pool face exists only for selection; it joins the published world only once actually resolved.
_Avoid_: system font db, fallback fonts (as a synonym for the pool)

**Published registry**:
The immutable snapshot mapping every known `FaceId` (loaded and resolved faces) to font data and metrics, read lock-free by the renderer and by session resolution.
_Avoid_: font registry (ambiguous with the candidate pool), minted registry

**Registry sync**:
The per-session-open step where a scratch compares the published registry's face count against its last-seen count and loads faces it has not seen yet. Keeps per-handle shapers aligned with the identity world without locking.
_Avoid_: epoch sync, epoch-pull, seed

**Session**:
One acquisition of a handle's exclusive shaper: registry snapshot + scratch, opened by `FontManager::shaper` and dropped before the frame's output is submitted.
