---
name: 'Error Handling'
description: 'Error handling conventions for Rust code in this workspace.'
applyTo: '**/*.rs'
---
# Error Handling

- Use `anyhow::Result` for application code.
- Add context to errors with `.context()`/`.with_context()` including relevant details (file paths, operations); return errors rather than fallback values.
- Don't do defensive programming; anything unexpected should lead to an error rather than being silently handled.
- Don't discard errors by matching only successful `Result` values; propagate or explicitly handle every error path.
