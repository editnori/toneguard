---
name: "CLI lint docs"
entrypoint: "dwg (default command)"
inputs:
  - "config path (layth-style.yml or override)"
  - "paths list"
  - "category toggles"
outputs:
  - "diagnostics to stdout (human or JSON)"
side_effects:
  - "reads config file"
  - "reads files from disk"
  - "writes JSON output when requested"
failure_modes:
  - "config parse error -> exit non-zero"
  - "no files found -> error"
  - "strict mode -> exit non-zero on warn threshold"
observability:
  - "stdout summary"
  - "JSON report when --json is set"
steps:
  - "Parse CLI args"
  - "Load config and apply overrides"
  - "Run repo hygiene checks"
  - "Collect files by path and profile"
  - "Analyze documents"
  - "Compute density and totals"
  - "Emit human or JSON report"
  - "Exit non-zero on strict threshold"
invariants:
  - "If --strict and density >= warn threshold, exit code is non-zero"
  - "Same inputs produce deterministic diagnostics"
  - "Repo checks run before document analysis unless disabled"
indirection_budget: 5
justifications:
  - item: "Analyzer"
    reason: "policy"
    evidence: "Centralizes deterministic rule evaluation"
  - item: "Repo checks layer"
    reason: "isolation"
    evidence: "Separates hygiene checks from doc analysis"
  - item: "Profiles"
    reason: "variation"
    evidence: "Allows rule overrides per doc type"
tags:
  - "cli"
  - "docs"
owners:
  - "toneguard"
language: "rust"
---

This flow represents the default CLI lint path for Markdown and text files.
