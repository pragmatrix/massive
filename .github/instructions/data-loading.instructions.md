---
name: 'Data Loading & Conversion'
description: 'Conventions for loading external data and converting between formats in this workspace.'
applyTo: '**/*.rs'
---
# Data Loading & Conversion

- When loading external formats, create intermediate deserialization types separate from runtime types, matching the source format structure, then convert to domain-appropriate runtime structures.
- Extract identifying information from source metadata (e.g., filenames, paths) when appropriate, returning errors if extraction fails rather than using defaults.
- For cross-boundary command/config changes, prefer explicit conversion layers and stage migrations so compiler errors guide the remaining integration work.
