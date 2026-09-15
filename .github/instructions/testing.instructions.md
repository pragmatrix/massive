---
name: 'Testing'
description: 'Testing conventions for Rust code in this workspace.'
applyTo: '**/*.rs'
---
# Testing

- Don't add tests unless explicitly asked.
- For bugs and regressions, always create a failing test first before implementing the fix; verify it fails for the right reason, then implement the fix and confirm the test passes.
- Place unit tests in a `#[cfg(test)] mod tests` in the same file; use `tests/` for integration tests.
- In tests: place test functions before helpers, create concise constructor helpers, prefer static data structures, and use helper functions for common value construction.
- For test assertions, derive `PartialEq` and `Eq` rather than implementing manually; prefer `Debug` over `Display` for output.
