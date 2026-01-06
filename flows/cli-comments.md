---
name: "CLI comment hygiene"
entrypoint: "dwg comments"
inputs:
  - "config path"
  - "paths list"
  - "--strip flag"
outputs:
  - "comment ratio report"
  - "optional stripped files"
side_effects:
  - "reads source files"
  - "writes updated files when --strip is set"
failure_modes:
  - "invalid config -> exit non-zero"
  - "strip on unsupported syntax -> skip"
observability:
  - "stdout report"
steps:
  - "Parse CLI args"
  - "Load config and comment policy"
  - "Collect eligible files"
  - "Analyze comment ratios"
  - "Optionally strip comments"
  - "Exit non-zero if policy exceeded"
invariants:
  - "No stripping occurs unless --strip is provided"
  - "Unsupported block comment syntaxes are skipped"
  - "Policy thresholds are enforced consistently"
indirection_budget: 4
justifications:
  - item: "Comment policy"
    reason: "policy"
    evidence: "Centralizes thresholds and allow/ignore globs"
  - item: "Syntax detection"
    reason: "isolation"
    evidence: "Keeps comment parsing per language"
tags:
  - "cli"
  - "hygiene"
owners:
  - "toneguard"
language: "rust"
---

Comment hygiene checks help avoid TODO sprawl and low-signal commentary.
