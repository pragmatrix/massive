# The desktop is a fractal of projects in slots

The desktop previously had exactly two levels: the `Desktop` (the implicit root
that fit all projects) holding top-level projects, each project owning a matrix
of launchers. The model now recurses: exactly one project is the root
(`DesktopTarget::Desktop` and `FocusDepth::Desktop` are removed); a project's
matrix is filled by **slots**, and each slot hosts either a launcher or another
nested project — never both. Nested projects are single-parented and appear in
exactly one placement. A launcher keeps its existing meaning: the configured
profile that spawns instances and owns a launcher mode. Nesting is arbitrary in
depth.

## Presentation and interaction are one depth cut

A project that is not the focused project presents exactly one slot level deep:
its slots render (a launcher slot without its instances, a project slot without
its nested slots) and the slots are only clickable — a click is a zoom-in
gesture. Nothing deeper is rendered or interactive. The focused project itself
behaves as before at every depth of its ladder. Rendering (content pruning) and
interaction (hit testing and focus routing) share this single rule, which is
what makes the model fractal rather than two ad-hoc behaviors; it generalizes
to arbitrary nesting depth without new vocabulary. Only the depth ladder
borrows the slot word. The overview depth ladder reads outermost first —
`Project, Row, Slot, Instance, Full Screen` (`Launcher` renamed to `Slot`,
`Desktop` removed): a rung's position counts the zoom-ins from the project
level, so the ladder composes across nesting — each zoom-in through a project
slot descends the newly focused project's ladder. There is no rung above the
root project's Project depth, and `ZoomOut` at that floor is a no-op (clamped;
repr `0` makes the floor the underflow guard).

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

## Vocabulary and addressing

Configuration addressing uses **address paths**: slash-separated names resolved
nearest-match per segment (ADR 0010's duplicate-name rule, applied per level).
The invoking instance's project is the implicit base for relative paths from
the CLI; a leading separator addresses from the root. `startup` persists an
address path. The configuration change vocabulary becomes slot-centric:
`AddSlot` (placement + content: launcher profile or project name),
`RemoveSlot`, `MoveSlot` (source; destination parent + placement; shift events
on both matrices), `SetStartup`. Replacement of slot content is remove + add,
not an in-place operation.