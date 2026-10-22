# massive

A UI framework with a 3D desktop: shell, desktop, views, key delivery and the IME.

## Terms

`GLOSSARY.md` defines the desktop's terms. Check it, including its `_Avoid_` lists, before naming a
type, function or concept in code or an ADR, and add the term there when you introduce one.

## Exceptional behaviors

A guard, redirect, or fallback that users could expect to work differently is decided in
`desktop/src/policy/when.rs`. Read ADR 0029 before adding or changing one.

## Tuning values

A size, duration, threshold, or easing curve that was picked by taste rather than derived lives in
`desktop/src/policy/constants.rs`, and a color lives in `desktop/src/policy/colors.rs`.

## Key bindings

`docs/keyboard-shortcuts.md` lists every key binding of the desktop, the shell and the terminal
client, plus planned and free keys. Consult it before choosing a key; update it in the same change
that adds, removes, or plans a binding, here or in a client.
