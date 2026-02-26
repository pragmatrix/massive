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

## Rust
- Prefer `derive_more` traits (Debug, Deref) over manual implementations.
- Do not import enum discriminants into scope; prefer qualified variants (e.g., `LauncherMode::Visor`).
- Use `pub` visibility by default. Only use `pub(crate)` when the containing module is already crate-public.
- Prefer adding fields to existing structs over creating parallel data structures.
- Use constructor functions and `derive_more::Deref` for newtype patterns.
- When implementing newtypes with `derive_more`, include `Copy` and `Clone` derives when the wrapped type supports them.
- Include complete state in events rather than deltas to provide full context to handlers.
- Prefer tuple parameters for semantically paired values (e.g., `(width, height)`) over separate scalar arguments when the values are always passed together.

## Safety & Quality
- Avoid unsafe or experimental APIs unless required.
- Preserve backwards compatibility unless instructed otherwise.
- When refactoring, don't add trait implementations that weren't present; prefer deriving over manual implementation.
- Keep one source of truth for mutable state; avoid mirrored caches and route reads through narrow accessors.
- For internal invariant violations, prefer explicit panics over silent fallback/continue paths.
- When code guarantees an invariant, avoid defensive fallback branches for that path; keep the direct path and fail explicitly if the invariant is violated.
- For purely defensive invariant checks on hot paths, prefer debug-only assertions to avoid unnecessary release-build work.

## Testing
- Don't add tests unless explicitly asked.
- In tests: place test functions before helpers, create concise constructor helpers to reduce verbosity, prefer static data structures, and use helper functions for common value construction patterns.
- For test assertions, derive `PartialEq` and `Eq` rather than implementing manually; prefer `Debug` over `Display` for output.

## Error Handling
- Use `anyhow::Result` for application code.
- Add context to errors with `.context()` or `.with_context()` including relevant details (file paths, operations); return errors rather than fallback values.
- Don't do defensive programming; anything unexpected should lead to an error rather than being silently handled.

## Data Loading & Conversion
- When loading data from external formats, create intermediate types for deserialization that are separate from runtime types.
- Design intermediate types to match the source format structure, then convert to domain-appropriate runtime structures.
- Extract identifying information from source metadata (e.g., filenames, paths) when appropriate, returning errors if extraction fails rather than using defaults.

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
