# Copilot Instructions

This file serves as the evolving knowledge base for working with this codebase.
Update it whenever you learn something new about the project's patterns, conventions, or receive feedback that should guide future behavior.

## Project
- Follow the existing project structure and idioms.
- Prefer small, self-contained changes unless explicitly asked for broader refactors.

## Code Style
- Match the surrounding code style.
- Keep functions small, clear, and deterministic.
- Avoid unnecessary dependencies.
- Do not add obvious comments that restate what the code clearly expresses.
- Only comment to explain non-obvious reasoning or intent.
- Before implementing a manual `Debug` trait, prefer using `derive_more`'s `Debug` with `#[debug(skip)]` attributes when appropriate.
- Limit qualification paths to at most 2 module levels (e.g., `mpsc::channel` not `tokio::sync::mpsc::channel`).
- Import types and modules to reduce path qualification in code.
- Order functions so that they call functions defined further down in the file (higher-level functions first, lower-level utilities last).
- Order types and structs by importance: public API first, then implementations, then private helper types last.
- Use `pub` visibility by default. Only use `pub(crate)` to limit visibility when the entire containing module is already crate-public.
- When adding new data that relates to an existing entity, prefer adding fields to the existing struct rather than creating parallel data structures (e.g., separate HashMaps keyed by the same ID).
- When multiple `Mutex` fields protect related data, consider consolidating them into a single `Mutex` around a state struct to reduce lock overhead and ensure atomic access.
- Look for opportunities to eliminate unnecessary wrapper types when they no longer serve a purpose.
- Prefer using constructor functions over struct literals when constructing types.
- For newtype patterns wrapping a single value (e.g., `struct Wrapper(T)`), use `derive_more::Deref` to enable ergonomic access instead of requiring `.0` everywhere.
- When implementing newtypes with `derive_more`, include `Copy` and `Clone` derives when the wrapped type supports them.
- When generating state transition events or similar sequences, include cumulative/complete state in each event rather than just the delta. This provides full context to event handlers.
- In data structures with paired values (like min/max bounds), group them logically: prefer `x: [f32; 2], y: [f32; 2]` over `min_x, min_y, max_x, max_y` for clarity.
- Prefer using standard library traits (`Add`, `Mul`, etc.) over creating custom traits when possible.
- When operations can be expressed using simpler primitives (e.g., subtraction as `a + (b * -1.0)`), avoid adding extra trait requirements.
- Use `Self::Variant` consistently in enum match statements rather than fully qualifying the enum name.
- Types that are private to a module don't need constructor functions - use struct literals directly.

## Safety & Quality
- Avoid unsafe or experimental APIs unless required.
- Add or update tests when modifying behavior.
- Preserve backwards compatibility unless instructed otherwise.
- When refactoring, don't add trait implementations (Clone, Debug, Default, etc.) that weren't present in the original code.
- If a trait can't be derived due to field constraints, investigate whether the trait is actually needed before implementing it manually.
- When writing tests with similar structure, create helper functions that format results as strings and test multiple cases in a single test function with one-line assertions rather than writing many verbose test functions.
- In tests, place test functions before helper functions they call to make the test structure immediately visible.
- Prefer `std::fmt::Debug` over `Display` for test output formatting, as it's more universally available and provides good default representations.
- For data types used in test assertions, derive `PartialEq` and `Eq` rather than implementing them manually.
- In test modules, create concise constructor helper functions to reduce verbosity in assertions (e.g., `rect(x, y, w, h)` instead of `Rect::new(Offset { dim: [x, y] }, Size { dim: [w, h] })`).

## Communication
- Explanations should be concise and strictly relevant.
- When unsure, ask clarifying questions before making assumptions.

## Documentation
- Markdown documentation updates to existing files are fine.
- Ask before creating new Markdown documentation files.
