---
name: "Flow audit"
entrypoint: "dwg flow audit"
inputs:
  - "config path"
  - "paths list"
  - "language filters"
outputs:
  - "flow audit report"
  - "optional JSON file"
side_effects:
  - "reads source files"
  - "writes report when --out is provided"
failure_modes:
  - "parse failures -> report with reduced confidence"
  - "missing flows -> error in flow check"
observability:
  - "stdout summary"
  - "JSON report"
steps:
  - "Load config"
  - "Optionally validate flow specs"
  - "Scan code files"
  - "Detect placeholders, lonely abstractions, pass-through chains"
  - "Summarize findings"
  - "Write JSON report"
invariants:
  - "Findings are deterministic for same inputs"
  - "Flow check errors do not prevent code audit"
  - "Findings include evidence strings"
indirection_budget: 5
justifications:
  - item: "Detector set"
    reason: "policy"
    evidence: "Enforces entropy guardrails across languages"
  - item: "Language adapters"
    reason: "variation"
    evidence: "Supports Rust, TypeScript, Python analysis"
tags:
  - "flow"
  - "audit"
owners:
  - "toneguard"
language: "rust"
---

Flow audit ties logic specs to static entropy checks.
