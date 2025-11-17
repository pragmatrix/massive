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

## Safety & Quality
- Avoid unsafe or experimental APIs unless required.
- Add or update tests when modifying behavior.
- Preserve backwards compatibility unless instructed otherwise.

## Communication
- Explanations should be concise and strictly relevant.
- When unsure, ask clarifying questions before making assumptions.

## Documentation
- Markdown documentation updates to existing files are fine.
- Ask before creating new Markdown documentation files.
