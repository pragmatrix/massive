# massive

A UI framework with a 3D desktop: shell, desktop, views, key delivery and the IME.

## Terms

`GLOSSARY.md` defines the desktop's terms. Check it, including its `_Avoid_` lists, before naming a
type, function or concept in code or an ADR, and add the term there when you introduce one.

## Exceptional behaviors

A guard, redirect, or fallback that users could expect to work differently is decided in
`desktop/src/policy/when.rs`. Read ADR 0029 before adding or changing one.

## Security

`docs/security.md` tracks the trust model, every security check, and the known gaps. A check that
guards what a process outside the desktop may do lives in `desktop/src/security/`, never in the
desktop policy. Read the document before touching the desktop socket, the CLI's inherited file
descriptor, or MCP, and update it in the same change.

## Tuning values

A size, duration, threshold, or easing curve that was picked by taste rather than derived lives in
`desktop/src/policy/constants.rs`, and a color lives in `desktop/src/policy/colors.rs`.

## Key bindings

`docs/keyboard-shortcuts.md` lists every key binding of the desktop, the shell and the terminal
client, plus planned and free keys. Consult it before choosing a key; update it in the same change
that adds, removes, or plans a binding, here or in a client.
