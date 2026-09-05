# Shared Instructions

This file holds the generic guidance shared across the projects that reference it. It is referenced from each project's `.github/copilot-instructions.md`; keep project-specific guidance in those files, not here.

Topic-specific conventions (testing, error handling, data loading) live in `.github/instructions/*.instructions.md` and apply automatically to matching files.

## Code Style
- Prefer small, self-contained changes unless broader refactors are requested; smaller diffs are easier to review and revert.
- Match surrounding code style; keep functions small, clear, and deterministic.
- Give each function, type, and module a single, focused responsibility.
- Avoid shallow forwarding functions; expose a composed capability directly or combine related work into a meaningful operation.
- Consolidate multiple exit points that return the same result when it improves readability.
- Comment only to explain non-obvious reasoning or intent; prefer concise, ideally one-line comments for conceptual/semantic blocks. Document the reason behind unusual behavior (cache invalidation, lifecycle ordering) so future readers don't "fix" it.
- Preserve existing comments during refactors unless inaccurate; update them when their rationale changes.
- Order functions high-level first, order their calls top-down by dependency (bottom-up in dependency order: a function calls only functions declared below it, so readers follow control flow upward toward the callee); order types by importance (public API first, private helpers last).
- When splitting large modules, extract low-coupling impl blocks first and preserve external imports via local re-exports in the parent module.

## Design Principles
- Apply the Single Responsibility Principle: each type, function, trait, and module should have one reason to change; separate policy from mechanism when they change independently.
- Prefer composition of structs, enums, and small functions over inheritance-style designs or large stateful objects.
- Use traits only at genuine substitution boundaries; keep them small and capability-focused, and require every implementation to honor the same behavioral contract.
- Apply the Interface Segregation Principle: depend on narrow, capability-focused interfaces rather than richer owner objects.
- Depend on stable domain abstractions rather than concrete infrastructure when multiple implementations or independent evolution justify the boundary; do not introduce traits speculatively.
- Make invalid states difficult to represent (enums, newtypes, constructors, private fields); enforce invariants at ownership boundaries.
- Prefer immutable values and explicit data flow; keep mutable state narrowly owned and avoid hidden temporal coupling.
- Extend behavior through existing composition points where practical, but prefer an exhaustive enum when the variant set is intentionally closed.
- Remove meaningful duplication, but tolerate small incidental repetition until a stable shared concept emerges.
- Choose the simplest design that satisfies current requirements; avoid abstractions for hypothetical future needs.

## Rust
- Prefer `derive_more` (Debug, Deref) and deriving traits over manual implementations when equivalent derives exist; derives are less error-prone and stay in sync with the type.
- Don't import enum discriminants into scope; prefer qualified variants (e.g., `LauncherMode::Visor`).
- Flatten `use` declarations into direct module-path groups; combine leaf imports sharing the exact module path. Keep grouped imports shallow; avoid multi-level brace nesting unless the file already uses that style.
- Import distinctive external types directly rather than fully qualified paths; keep original names unless they conflict, then use a clear alias.
- Use `pub` by default; use `pub(crate)` only when the containing module is already crate-public. Control visibility at module boundaries.
- Prefer adding fields to existing structs over parallel data structures; parallel fields drift out of sync.
- Use constructor functions and `derive_more::Deref` for newtypes; include `Copy` and `Clone` derives when the wrapped type supports them.
- Include complete state in events rather than deltas to give handlers full context.
- Prefer tuple parameters for semantically paired values (e.g., `(width, height)`) always passed together.
- Prefer accessors returning references; callers clone only when they need ownership.
- Prefer named structs over tuple returns when ordering or intent may be ambiguous.
- Prefer behavior-named capability methods over exposing raw mode enums to higher-level callers.
- In nested Cargo workspaces, keep shared dependency versions aligned across manifests before applying API migration fixes.
- For graphics crate major upgrades, prefer release-note-driven API migrations first (constructor changes, enum-return replacements, option-wrapped descriptor fields) before broader refactors.

## Safety & Quality
- Avoid unsafe or experimental APIs unless required; preserve backwards compatibility unless instructed otherwise.
- When refactoring, don't add trait implementations that weren't present; prefer deriving over manual implementation.
- Keep one source of truth for mutable state; avoid mirrored caches and route reads through narrow accessors.
- Represent state with the same lifetime and update boundary as one value, instead of parallel optional fields.
- When presentation can be derived from authoritative interaction, layout, and environment state, apply it as an immutable value through an ordered effect instead of mirroring mode flags.
- Filter inexpensive eligibility conditions on source values before constructing derived state requiring mutable access or expensive work.
- When a scene object's identity persists across updates, retain its handle and use `update_if_changed`; create or drop the handle only when the object appears or disappears.
- For mode-specific interaction behavior, prefer a focused second-pass evaluation over broad global rule changes affecting unrelated paths.
- Preserve source identity when transforming input events; synthetic follow-up events should retain the originating identity.
- For internal invariant violations, prefer explicit panics over silent fallback/continue paths; when code guarantees an invariant, keep the direct path and fail explicitly rather than adding defensive fallback branches.
- For purely defensive invariant checks on hot paths, prefer debug-only assertions to avoid release-build work.
- When internal APIs become infallible, propagate that contract through helpers and traits and remove Option-based branching unless absence is still a real domain state.
- Treat unnecessary `Option` state as defensive programming; prefer concrete state when initialization and invariants guarantee a value.
- Carry out computations (and writes/updates) as precisely as possible: only when needed, at the narrowest branch where the result is consumed.
- Prefer proper platform-native solutions over UI-level workarounds or quick fixes.
- Keep invariant checks and mode gating at one layer where practical.
- When structure guarantees a concrete target type, convert at the boundary instead of carrying optional identities through lower-level APIs.
- Cache repeated expensive state requests at the caller when the underlying operation may be non-trivial.
- Prefer native, user-remappable command routing over hardcoded shortcut matching when platform conventions support remapping.
- When refactoring eventful flows, extract pure target/decision helpers first and keep side-effect dispatch ordering unchanged until tests lock transition semantics.
- When adding hierarchical layout metadata, compose effective values across the full ancestor path at absolute-placement boundaries instead of relying only on the target-local value.
- For runtime-driven presenter animation, keep mutable animation state in a dedicated movement value, retain its `Movement` handle and scene outputs on the presenter, and capture cloned scene handles in the movement callback.

## Communication
- Explanations should be concise and strictly relevant.
- When unsure, ask clarifying questions before making assumptions.
- When a request leaves a material design choice open, suggest viable solutions and wait for direction before implementing one.

## Continuous Learning
- After completing meaningful work, update this file with high-level, reusable guidance learned from the task.
- When the user says "remember", record the applicable guidance in this file as well as persistent memory.
- Keep additions general (patterns, principles, decision heuristics), not task- or file-specific details.
- Do not add project-specific implementation facts (feature behavior, constants, file-local decisions) or one-off, narrowly scoped rules.
- Prefer small, incremental updates over large rewrites; avoid duplicating or restating existing guidance.

## Documentation
- Don't add documentation with examples unless explicitly asked; markdown updates to existing files are fine.
- Ask before creating new Markdown documentation files.
- Maintain concise comment-based documentation for every type and module, explaining why each exists.
