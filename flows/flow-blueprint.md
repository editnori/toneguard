---
name: "Flow blueprint"
entrypoint: "dwg flow blueprint"
inputs:
  - "config path"
  - "paths list"
  - "output format (json or jsonl)"
outputs:
  - "blueprint report (nodes + edges)"
  - "optional JSON/JSONL file"
side_effects:
  - "reads source files"
  - "writes report when --out is provided"
failure_modes:
  - "unsupported format -> error"
  - "unreadable file -> recorded in errors list"
observability:
  - "stdout JSON/JSONL"
  - "optional output file"
steps:
  - "Load config"
  - "Collect code files (respect ignore globs)"
  - "Extract import/mod/use edges per file"
  - "Resolve edges to scanned files when possible"
  - "Write report"
invariants:
  - "Output is deterministic for the same repo state"
  - "Every resolved edge points to an existing scanned node"
  - "Unreadable files do not abort the entire run"
indirection_budget: 5
justifications:
  - item: "Blueprint report"
    reason: "policy"
    evidence: "Provides a repo-wide map for code review and LLM prompts"
tags:
  - "flow"
  - "blueprint"
owners:
  - "toneguard"
language: "rust"
---

Flow blueprint builds a repo-wide graph of files and dependencies.
