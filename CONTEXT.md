# Massive Desktop Interaction Context

This context defines the interaction and presentation language for desktop instance layout and visibility behavior. It exists to keep behavior terms consistent across layout, hit testing, and rendering discussions.

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
