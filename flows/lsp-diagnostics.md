---
name: "LSP live diagnostics"
entrypoint: "dwg-lsp server"
inputs:
  - "workspace root"
  - "config path"
  - "open document text"
outputs:
  - "live diagnostics in editor"
side_effects:
  - "reads workspace files"
  - "sends diagnostics to editor client"
failure_modes:
  - "config load error -> default config"
  - "missing bundled binary -> fallback to PATH"
observability:
  - "LSP log output"
steps:
  - "Start LSP server"
  - "Resolve config path"
  - "Load analyzer"
  - "Track document changes"
  - "Analyze content on debounce"
  - "Publish diagnostics"
  - "Handle config updates"
invariants:
  - "Diagnostics are based on latest document version"
  - "Analyzer reload respects config changes"
  - "No file writes from LSP"
indirection_budget: 5
justifications:
  - item: "Analyzer cache"
    reason: "policy"
    evidence: "Keeps analyzer hot for fast diagnostics"
  - item: "Debounce"
    reason: "isolation"
    evidence: "Prevents rapid re-analysis during typing"
tags:
  - "lsp"
  - "editor"
owners:
  - "toneguard"
language: "rust"
---

The LSP keeps diagnostics live without manual CLI runs.
