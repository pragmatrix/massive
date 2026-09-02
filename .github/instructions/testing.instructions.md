---
name: 'Testing'
description: 'Testing conventions for Rust code in this workspace.'
applyTo: '**/*.rs'
---
# Testing

- Don't add tests unless explicitly asked.
- For behavioral feedback where subtle update-stream/order correctness is at risk, ask for (or add) a failing regression test first before implementation changes.
- In tests: place test functions before helpers, create concise constructor helpers, prefer static data structures, and use helper functions for common value construction.
- For test assertions, derive `PartialEq` and `Eq` rather than implementing manually; prefer `Debug` over `Display` for output.
