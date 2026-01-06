---
name: "Flow proposal"
entrypoint: "dwg flow propose"
inputs:
  - "config path"
  - "paths list"
  - "language filters"
outputs:
  - "Markdown review artifact"
  - "optional output file"
side_effects:
  - "reads source files"
  - "writes proposal when --out is provided"
failure_modes:
  - "parse failures -> reduced coverage"
  - "missing flows -> errors in flow check (unless skipped)"
observability:
  - "stdout Markdown"
  - "saved proposal file"
steps:
  - "Load config"
  - "Optionally validate flow specs"
  - "Scan code files"
  - "Group findings and render Markdown"
  - "Write proposal when --out is provided"
invariants:
  - "Proposal is deterministic for same inputs"
  - "Proposal is evidence-backed (paths/lines/symbols)"
  - "Flow check errors do not prevent code audit"
indirection_budget: 5
justifications:
  - item: "Proposal artifact"
    reason: "policy"
    evidence: "Creates a concrete document humans can point at during review"
tags:
  - "flow"
  - "proposal"
owners:
  - "toneguard"
language: "rust"
---

This command generates a reviewable Markdown artifact from flow checks + entropy detectors.
