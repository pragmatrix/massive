# Copilot Instructions

This file serves as the evolving knowledge base for working with this codebase.
Update it whenever you learn something new about the project's patterns, conventions, or receive feedback that should guide future behavior.

## Project-Specific: massive
- This workspace is a Cargo workspace rooted at [Cargo.toml](../Cargo.toml) with core crates under top-level folders like [scene](../scene), [renderer](../renderer), [desktop](../desktop), and [shell](../shell).
- The source-of-truth architecture overview for scene/object lifetime is in [scene/src/lib.rs](../scene/src/lib.rs). Read this before changing scene handle or change-propagation behavior.
- Use [README.md](../README.md) for example entry points and expected demo behavior instead of inferring from code paths.
- Treat [examples/code/rust-analyzer](../examples/code/rust-analyzer) and [examples/markdown/inlyne](../examples/markdown/inlyne) as imported upstream projects; do not modify them unless explicitly asked.
- Prefer changes in first-party crates listed in [Cargo.toml](../Cargo.toml) workspace members. Keep changes scoped to the relevant crate; avoid cross-crate refactors unless the task explicitly requires them.
- Prefer `cargo build` for broad compile validation. If a task is scoped to one crate, prefer crate-targeted validation before workspace-wide commands.
- Demo runs from [README.md](../README.md): `cargo run --release --example code` and `cargo run --release --example markdown`.
- WASM example workflows live in [justfile](../justfile), including `trunk serve --example markdown --port 8888 --open` and release build targets.
- Architecture anchors: scene graph and handle model in [scene/src/lib.rs](../scene/src/lib.rs), [scene/src/handle.rs](../scene/src/handle.rs), [scene/src/change.rs](../scene/src/change.rs); fluent scene ergonomics in [scene/src/ergonomics.rs](../scene/src/ergonomics.rs); desktop orchestration and event routing in [desktop/src/lib.rs](../desktop/src/lib.rs); platform split (native vs wasm) in [shell/Cargo.toml](../shell/Cargo.toml) and [animation/src/lib.rs](../animation/src/lib.rs). Prefer linking to these files in explanations instead of duplicating architectural prose.
- When domain terms or architectural trade-offs are resolved, prefer recording them in CONTEXT.md and docs/adr/* over creating other new Markdown documentation files.

## Shared Guidance
The generic guidance for code style, design principles, Rust conventions, safety & quality, testing, error handling, data loading, communication, continuous learning, and documentation lives in [shared-instructions.md](./shared-instructions.md). Keep generic guidance there and project-specific guidance in this file.

Topic-specific conventions (testing, error handling, data loading) live in `.github/instructions/*.instructions.md` and apply automatically to matching files.
