# Copilot Instructions

This file serves as the evolving knowledge base for working with this codebase.
Update it whenever you learn something new about the project's patterns, conventions, or receive feedback that should guide future behavior.

## Code Style
- Prefer small, self-contained changes unless explicitly asked for broader refactors.
- Match the surrounding code style.
- Keep functions small, clear, and deterministic.
- Avoid multiple exit points that return the same result; consolidate them when it improves readability.
- Comment only to explain non-obvious reasoning or intent.
- Order functions high-level first, utilities last; order types by importance (public API first, private helpers last).
- Prefer directory submodules with `mod.rs` over sibling `foo.rs` submodule files when introducing new submodule trees.
- When splitting large modules, extract low-coupling impl blocks first and preserve existing external imports via local re-exports in the parent module.

## Rust
- Prefer deriving traits over manual implementations when equivalent derives are available.
- Prefer qualified enum variants when it improves clarity over imported discriminants.
- Structure imports with common-root `use` lines (for example `use std::sync::Arc;`) rather than nested hierarchical `use` trees.
- Keep grouped imports shallow; avoid multi-level brace nesting in `use` statements unless the surrounding file already consistently uses that style.
- Use `pub` visibility by default. Only use `pub(crate)` when the containing module is already crate-public.
- When replacing dependency re-exports with direct upstream imports, verify whether wrapper/helper types came from the old dependency; if upstream does not provide them, keep compatibility by defining small local boundary types.
- Prefer adding fields to existing structs over creating parallel data structures.
- Use constructor functions and derive helpers for newtype patterns.
- When implementing newtypes, include `Copy` and `Clone` when the wrapped type supports them.
- Include complete state in events rather than deltas to provide full context to handlers.
- Prefer grouping semantically paired values into a single parameter or type when they are always used together.
- Use cohesive domain types as API boundaries when related values are expected to move together.
- When a domain struct already models paired values, prefer it over tuple payloads in change streams and method signatures.
- When a cohesive domain struct is the canonical state, prefer a single accessor returning that struct over parallel field-specific accessors.
- Prefer named structs over tuple returns when ordering or intent may be ambiguous.
- For small paired-value structs, prefer constructors at call sites over repeated field-literal initialization.
- Prefer behavior-named capability methods over exposing raw mode enums to higher-level callers.
- Keep mode-specific decisions behind a single owning abstraction instead of splitting them across multiple caller-side passes.
- For graphics crate major upgrades, prefer release-note-driven API migrations first (constructor changes, enum-return replacements, and option-wrapped descriptor fields) before broader refactors.

## Safety & Quality
- Avoid unsafe or experimental APIs unless required.
- Preserve backwards compatibility unless instructed otherwise.
- When refactoring, don't add trait implementations that weren't present; prefer deriving over manual implementation.
- For event transition summaries used by side effects, collect all relevant transition payloads rather than stopping at the first match.
- Prefer proper platform-native solutions over UI-level workarounds or quick fixes.
- Keep one source of truth for mutable state; avoid mirrored caches and route reads through narrow accessors.
- When removing redundant scene graph nodes, preserve visual alignment by moving centering/offset math into local geometry when possible instead of introducing replacement transform handles.
- For transient UI indicators (hover/focus highlights), derive visibility/target from current resolved state rather than only from enter/exit edge events.
- For context-specific behavior, prefer targeted follow-up evaluation over broad global rule changes that affect unrelated paths.
- When a generic pass applies fallback state, recompute context-specific state immediately afterward for impacted entities.
- For visual side effects derived from state transitions, prefer computing them in the centralized effects/update phase using previous/current state snapshots instead of duplicating eager updates across input and command paths.
- Keep invariant gating at a single layer where practical; avoid repeating identical mode/eligibility checks across caller and callee.
- When an operation must not emit follow-up commands, model it as `Result<()>` and enforce the invariant at the forwarding boundary.
- For internal invariant violations, prefer explicit panics over silent fallback/continue paths.
- When code guarantees an invariant, avoid defensive fallback branches for that path; keep the direct path and fail explicitly if the invariant is violated.
- When structure guarantees a concrete target type, convert at the boundary instead of carrying optional identities through lower-level APIs.
- For purely defensive invariant checks on hot paths, prefer debug-only assertions to avoid unnecessary release-build work.
- For platform-specific commands, detect shortcuts where aggregated input state is available and keep mutations in a platform abstraction layer.
- When multiple transient affordances represent the same interaction mode, keep them behind one shared state instead of parallel flags.
- Cache repeated expensive state requests at the caller when the underlying operation may be non-trivial.
- Prefer native, user-remappable command routing over hardcoded shortcut matching when platform conventions support remapping.
- When refactoring eventful flows, extract pure target/decision helpers first and keep side-effect dispatch ordering unchanged until tests lock transition semantics.

## Testing
- Don't add tests unless explicitly asked.
- For behavioral feedback where subtle update-stream/order correctness is at risk, ask for (or add) a failing regression test first before implementation changes.
- In tests: place test functions before helpers, create concise constructor helpers to reduce verbosity, prefer static data structures, and use helper functions for common value construction patterns.
- For test assertions, derive `PartialEq` and `Eq` rather than implementing manually; prefer `Debug` over `Display` for output.

## Error Handling
- Use `anyhow::Result` for application code.
- Add context to errors with `.context()` or `.with_context()` including relevant details (file paths, operations); return errors rather than fallback values.
- Don't do defensive programming; anything unexpected should lead to an error rather than being silently handled.
- For recursive tree searches, prefer `Option` in recursive helpers and convert to `Result` once at the public entry point.

## Data Loading & Conversion
- When loading data from external formats, create intermediate types for deserialization that are separate from runtime types.
- Design intermediate types to match the source format structure, then convert to domain-appropriate runtime structures.
- Extract identifying information from source metadata (e.g., filenames, paths) when appropriate, returning errors if extraction fails rather than using defaults.
- For cross-crate command flows, define transport-layer command types in the upstream crate and perform explicit conversion at the consumer boundary.
- For configuration-format migrations, refactor domain types first, then adapt readers and conversion layers so compiler errors clearly guide the remaining integration changes.

## Communication
- Explanations should be concise and strictly relevant.
- When unsure, ask clarifying questions before making assumptions.

## Continuous Learning
- After completing meaningful work, update this file with high-level, reusable guidance learned from the task.
- Keep additions general (patterns, principles, decision heuristics), not task- or file-specific details.
- Do not add project-specific implementation facts (feature behavior, constants, file-local decisions); keep guidance broadly reusable.
- Prefer small, incremental updates over large rewrites, and avoid duplicating or restating existing guidance.

## Documentation
- Don't add documentation with examples unless explicitly asked.
- Markdown documentation updates to existing files are fine.
- Ask before creating new Markdown documentation files.
