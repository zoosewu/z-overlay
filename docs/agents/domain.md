# Domain Docs

Before exploring, read `CONTEXT.md` at the repo root and ADRs under `docs/adr/` that touch the work. If a `CONTEXT-MAP.md` exists, follow it to the relevant context instead. Missing files require no action; `/domain-modeling` creates them lazily when terms or decisions are resolved.

This is a single-context repository:

```text
/
|-- CONTEXT.md
|-- docs/adr/
`-- src/
```

Use glossary terms exactly as defined in `CONTEXT.md`. Surface conflicts with an existing ADR instead of silently overriding it.
