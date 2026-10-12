# Keyboard focus state lives beside the event router, not in it

## Status: accepted

The event router holds the keyboard focus next to the pointer focus, as one generic target type. It also saves and restores the focus when the window loses and regains focus, and it addresses keys to the focused target. ADR 0020 already states that the router keeps only pointer focus, and key delivery walks the focus path without using the router's target. The keyboard focus can only hold a target that selects itself (ADR 0019), while the pointer focus must hold any hit surface. One type parameter cannot express both.

The keyboard focus state moves into its own **keyboard focus** component that, like the router, knows no hierarchy:

- **It owns values only.** It holds the keyboard-focused target as a focusable target, the window focus with the target to restore, and the input focus (ADR 0020). Its operations return a focus change, and they have no effects of their own. Whether the input focus still shows the hover outline becomes one of its rules.
- **The router keeps pointer focus and hit testing.** For keys, IME, modifier changes and window focus changes it emits steps without a keyboard target. Modifier changes still reach both the keyboard focus and a different pointer target; the desktop makes that comparison now.
- **The desktop system orchestrates.** It turns router steps into keyboard focus calls and applies the resulting change exactly as today: focus transitions to views, launcher relayout, the camera move, and remembering the project focus slot. Focus policies, such as which target navigation, zoom or a stopping instance focuses, stay in the desktop system, because they need the hierarchy.
- **A restored window focus is still a focus change.** It goes through the same change as any other focus change, so its effects run.

## Considered options

- *Keep the keyboard focus in the router and give it a second type parameter.* Rejected: the router would have to know the selection rule, because a pointer press becomes a keyboard focus request, and it would compare the two types when it forwards modifier changes.
- *Move the keyboard focus into the desktop system as a field.* Rejected: the desktop system is already the largest component, and the focus rules (window restore, input-focus validity) would again sit between unrelated state instead of being testable as values.

## Consequences

- The desktop system loses state, not behaviour. The effects of a focus change and the focus policies are inherent to the hierarchy and remain there.
- The keyboard focus is typed as a focusable target at its source, so a focus on a header, matrix or title bar becomes unrepresentable instead of an assertion.
- The input focus moves with the keyboard focus, because whether it is still valid depends on the keyboard focus.
