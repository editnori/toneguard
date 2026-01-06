---
name: "CLI calibration"
entrypoint: "dwg calibrate"
inputs:
  - "config path"
  - "paths list (good samples)"
outputs:
  - "calibration.yml file"
side_effects:
  - "reads sample files"
  - "writes calibration output"
failure_modes:
  - "no files found -> error"
  - "invalid YAML -> error"
observability:
  - "stdout summary"
steps:
  - "Parse CLI args"
  - "Load config"
  - "Collect sample files"
  - "Analyze samples"
  - "Compute density and sentence stats"
  - "Write calibration output"
  - "Print suggestions"
invariants:
  - "Calibration output reflects only provided samples"
  - "Suggested thresholds derived from density stats"
  - "Deterministic results for same inputs"
indirection_budget: 4
justifications:
  - item: "Calibration output file"
    reason: "policy"
    evidence: "Captures tuned thresholds separately from base config"
  - item: "Analyzer"
    reason: "reuse"
    evidence: "Reuses existing rule engine to evaluate samples"
tags:
  - "cli"
  - "calibration"
owners:
  - "toneguard"
language: "rust"
---

Calibration learns from good writing samples to tune thresholds.
