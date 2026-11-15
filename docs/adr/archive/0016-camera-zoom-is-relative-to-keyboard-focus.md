# Camera zoom is relative to keyboard focus

**Status: superseded by [ADR 0017](./0017-zoom-depth-is-counted-from-the-root.md)**

Camera zoom is a runtime count of outward framing steps resolved from the keyboard-focused target, rather than an independently stored camera target or a fixed focus-depth enum. Keyboard navigation changes focus while preserving that count, clamped to the levels available on the new target's hierarchy path; an outward step never decreases camera distance, even when its frame bounds are narrower. Each project remembers one immediate slot on the most recently focused path; nested projects continue the path, and a launcher's existing instance anchor selects its instance. This keeps camera framing coupled to the user's current focus while retaining the same relative zoom through navigation.

This decision concerns camera framing and focus restoration only; ADR 0011's presentation and interaction depth-cut decision is unchanged.
