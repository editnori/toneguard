---
name: "Flow call graph"
entrypoint: "dwg flow callgraph"
inputs:
  - "config path"
  - "paths list"
  - "output format (json or jsonl)"
  - "max calls per function"
  - "resolved-only toggle"
outputs:
  - "call graph report (function/method nodes + call edges)"
  - "hub/orphan/sink/source stats"
  - "optional JSON/JSONL file"
side_effects:
  - "reads Rust/TS/JS/Py source files"
  - "writes report when --out is provided"
failure_modes:
  - "unsupported format -> error"
  - "unreadable file -> recorded in errors list"
  - "unparseable Rust file -> recorded in errors list"
observability:
  - "stdout JSON/JSONL"
  - "optional output file"
steps:
  - "Load config"
  - "Collect code files (respect ignore globs)"
  - "Index functions and methods"
  - "Extract call sites per function body"
  - "Resolve calls to unique targets when possible"
  - "Compute degree stats (hubs/orphans/sinks/sources)"
  - "Write report"
invariants:
  - "Output is deterministic for the same repo state"
  - "Every resolved edge points to an existing node"
  - "Calls only resolve when the target is unique"
  - "Parse errors do not abort the entire run"
indirection_budget: 5
justifications:
  - item: "Call graph report"
    reason: "policy"
    evidence: "Adds a function-level view to complement file-level blueprint edges"
tags:
  - "flow"
  - "callgraph"
owners:
  - "toneguard"
language: "rust"
---

Flow call graph builds a function-level call graph for Rust, TypeScript/JavaScript, and Python.
It resolves edges conservatively (only when the callee maps to a unique target).
