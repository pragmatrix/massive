# Copilot Instructions

This file serves as the evolving knowledge base for working with this codebase.
Update it whenever you learn something new about the project's patterns, conventions, or receive feedback that should guide future behavior.

## Project Orientation
- This workspace is a Cargo workspace rooted at [Cargo.toml](../Cargo.toml) with core crates under top-level folders like [scene](../scene), [renderer](../renderer), [desktop](../desktop), and [shell](../shell).
- The source-of-truth architecture overview for scene/object lifetime is in [scene/src/lib.rs](../scene/src/lib.rs). Read this before changing scene handle or change-propagation behavior.
- Use [README.md](../README.md) for example entry points and expected demo behavior instead of inferring from code paths.

## Workspace Boundaries
- Treat [examples/code/rust-analyzer](../examples/code/rust-analyzer) and [examples/markdown/inlyne](../examples/markdown/inlyne) as imported upstream projects; do not modify them unless explicitly asked.
- Prefer changes in first-party crates listed in [Cargo.toml](../Cargo.toml) workspace members.
- Keep changes scoped to the relevant crate. Avoid cross-crate refactors unless the task explicitly requires them.

## Build, Run, and Validation
- Prefer `cargo build` for broad compile validation.
- Demo runs from [README.md](../README.md):
	- `cargo run --release --example code`
	- `cargo run --release --example markdown`
- WASM example workflows live in [justfile](../justfile), including `trunk serve --example markdown --port 8888 --open` and release build targets.
- If a task is scoped to one crate, prefer crate-targeted validation before workspace-wide commands.

## Architecture Anchors
- Scene graph and handle model: [scene/src/lib.rs](../scene/src/lib.rs), [scene/src/handle.rs](../scene/src/handle.rs), [scene/src/change.rs](../scene/src/change.rs)
- Fluent scene ergonomics: [scene/src/ergonomics.rs](../scene/src/ergonomics.rs)
- Desktop orchestration and event routing: [desktop/src/lib.rs](../desktop/src/lib.rs)
- Platform split (native vs wasm): [shell/Cargo.toml](../shell/Cargo.toml), [animation/src/lib.rs](../animation/src/lib.rs)
- Prefer linking to these files in explanations instead of duplicating architectural prose.

## Code Style
- Prefer small, self-contained changes unless explicitly asked for broader refactors.
- Match the surrounding code style.
- Keep functions small, clear, and deterministic.
- Avoid multiple exit points that return the same result; consolidate them when it improves readability.
- Comment only to explain non-obvious reasoning or intent.
- Prefer concise, ideally one-line comments for conceptual or semantic blocks inside functions.
- Order functions high-level first, utilities last; order types by importance (public API first, private helpers last).
- When splitting large modules, extract low-coupling impl blocks first and preserve existing external imports via local re-exports in the parent module.

## Rust
- Prefer deriving traits over manual implementations when equivalent derives are available.
- Prefer qualified enum variants when it improves clarity over imported discriminants.
- Structure imports with common-root `use` lines (for example `use std::sync::Arc;`) rather than nested hierarchical `use` trees.
- Keep grouped imports shallow; avoid multi-level brace nesting in `use` statements unless the surrounding file already consistently uses that style.
- Use `pub` visibility by default. Only use `pub(crate)` when the containing module is already crate-public.
- Prefer adding fields to existing structs over creating parallel data structures.
- Use constructor functions and derive helpers for newtype patterns.
- When implementing newtypes, include `Copy` and `Clone` when the wrapped type supports them.
- Prefer named structs over tuple returns when ordering or intent may be ambiguous.
- Prefer behavior-named capability methods over exposing raw mode enums to higher-level callers.
- For graphics crate major upgrades, prefer release-note-driven API migrations first (constructor changes, enum-return replacements, and option-wrapped descriptor fields) before broader refactors.

## Safety & Quality
- Avoid unsafe or experimental APIs unless required.
- Preserve backwards compatibility unless instructed otherwise.
- When refactoring, don't add trait implementations that weren't present; prefer deriving over manual implementation.
- Avoid redundant work; only perform writes, recomputations, or updates when they are necessary for correctness.
- Prefer proper platform-native solutions over UI-level workarounds or quick fixes.
- Keep one source of truth for mutable state, and prefer deriving computed values at read boundaries over maintaining mirrored caches.
- Keep invariant checks and mode gating at one layer where practical.
- For internal invariant violations, prefer explicit panics over silent fallback/continue paths.
- When code guarantees an invariant, avoid defensive fallback branches for that path; keep the direct path and fail explicitly if the invariant is violated.
- When structure guarantees a concrete target type, convert at the boundary instead of carrying optional identities through lower-level APIs.
- For purely defensive invariant checks on hot paths, prefer debug-only assertions to avoid unnecessary release-build work.
- Cache repeated expensive state requests at the caller when the underlying operation may be non-trivial.
- Prefer native, user-remappable command routing over hardcoded shortcut matching when platform conventions support remapping.
- When refactoring eventful flows, extract pure target/decision helpers first and keep side-effect dispatch ordering unchanged until tests lock transition semantics.
- When adding hierarchical layout metadata, compose effective values across the full ancestor path at absolute-placement boundaries instead of relying only on the target-local value.

## Testing
- Don't add tests unless explicitly asked.
- For behavioral feedback where subtle update-stream/order correctness is at risk, ask for (or add) a failing regression test first before implementation changes.
- In tests, keep test cases first and helpers below; use concise helpers to reduce verbosity.
- For test assertions, derive `PartialEq` and `Eq` rather than implementing manually; prefer `Debug` over `Display` for output.

## Error Handling
- Use `anyhow::Result` for application code.
- Add context to errors with `.context()` or `.with_context()` including relevant details (file paths, operations); return errors rather than fallback values.
- Don't do defensive programming; anything unexpected should lead to an error rather than being silently handled.

## Data Loading & Conversion
- When loading data from external formats, create intermediate types for deserialization that are separate from runtime types.
- Design intermediate types to match the source format structure, then convert to domain-appropriate runtime structures.
- Extract identifying information from source metadata (e.g., filenames, paths) when appropriate, returning errors if extraction fails rather than using defaults.
- For cross-boundary command/config changes, prefer explicit conversion layers and stage migrations so compiler errors guide the remaining integration work.

## Communication
- Explanations should be concise and strictly relevant.
- When unsure, ask clarifying questions before making assumptions.

## Continuous Learning
- After completing meaningful work, update this file with high-level, reusable guidance learned from the task.
- Keep additions general (patterns, principles, decision heuristics), not task- or file-specific details.
- Do not add project-specific implementation facts (feature behavior, constants, file-local decisions); keep guidance broadly reusable.
- Do not add one-off, narrowly scoped rules that only apply to a single recent change.
- Prefer small, incremental updates and avoid duplicating or restating existing guidance.

## Documentation
- Don't add documentation with examples unless explicitly asked.
- Markdown documentation updates to existing files are fine.
- Ask before creating new Markdown documentation files, except for CONTEXT.md and docs/adr/* when domain terms or architectural trade-offs are resolved.
