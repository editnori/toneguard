---
name: "Flow blueprint diff"
entrypoint: "dwg flow blueprint diff"
inputs:
  - "before blueprint (json)"
  - "after blueprint (json)"
  - "format (json or md)"
  - "optional mapping file (yaml)"
outputs:
  - "diff report (added/removed nodes + edges)"
  - "mapping template for removed files"
side_effects:
  - "reads blueprint snapshots"
  - "writes diff output when --out is provided"
  - "writes mapping template when --write-mapping is provided"
failure_modes:
  - "missing/invalid snapshot -> error"
  - "unsupported format -> error"
  - "mapping check fails -> exit non-zero"
observability:
  - "stdout JSON/Markdown"
  - "optional output files"
steps:
  - "Load before/after blueprint snapshots"
  - "Compute added/removed nodes"
  - "Compute added/removed resolved edges (ignore line numbers)"
  - "Suggest move/rename candidates from graph neighborhoods"
  - "Emit a mapping template for removed files"
  - "Optionally validate a user-provided mapping file"
invariants:
  - "Diff output is deterministic for the same snapshots"
  - "Removed nodes == before.nodes - after.nodes"
  - "Edge diff ignores source line numbers"
  - "If --require-mapping is set, the command exits non-zero when any removed node is unmapped"
indirection_budget: 5
justifications:
  - item: "Blueprint diff + mapping"
    reason: "policy"
    evidence: "Acts as a refactor protector: every deletion/move must be explained or mapped"
tags:
  - "flow"
  - "blueprint"
  - "diff"
owners:
  - "toneguard"
language: "rust"
---

Blueprint diff compares two blueprint snapshots and produces a small, reviewable diff.
It also emits a mapping template so refactors can't silently delete behavior.
