# ToneGuard VS Code Extension

ToneGuard flags common AI-style writing patterns in Markdown and text files. It also ships flow tooling (audit, blueprint, CFG) for code review.

## Quick start

1. Install the extension.
2. Open a folder.
3. Open the ToneGuard view (Activity Bar icon) and click **Run**.

## What you get

- Live diagnostics for supported file types (Problems panel + inline squiggles)
- Dashboard (Review, Findings, Organizer, Settings)
- Flow Map (Blueprint graph + clusters, Calls view, CFG viewer with copyable Mermaid)
- Markdown Preview (Mermaid render, syntax highlight, HTML/Confluence/PDF export)
- Optional skill installer (Cursor / Claude Code / Codex)

## Reports

The recommended review writes files under `reports/`:

- `reports/markdown-lint.json`
- `reports/flow-audit.json`
- `reports/flow-proposal.md`
- `reports/flow-index.json` (best-effort)
- `reports/flow-blueprint.json` (best-effort)
- `reports/flow-callgraph.json` (best-effort)
- `reports/flow-blueprint-diff.json` (best-effort)
- `reports/flow-blueprint-mapping.yml` (best-effort)

## LLM review helpers

- Dashboard: **Copy bundle** (includes report paths + small samples)
- Flow Map: **Copy packet** (includes blueprint/callgraph context for the current selection)

## Configuration

ToneGuard looks for a workspace config first (`layth-style.yml`, `.toneguard.yml`). If none exists, it uses the bundled defaults.

Useful settings:

- `dwg.configPath`: path to a custom config file
- `dwg.profile`: force a profile for all files
- `dwg.strict`: fail on warn-level density
- `dwg.noRepoChecks`: skip repo-wide checks
- `dwg.uiTheme`: theme for Dashboard + Flow Map

## Commands

Open the Command Palette and run:

- `ToneGuard: Run Recommended Review`
- `ToneGuard: Open Flow Map`
- `ToneGuard: Flow Audit Workspace`
- `ToneGuard: Generate Flow Proposal (Markdown)`
- `ToneGuard: Open Markdown Preview`

You can also right-click a markdown file (or tab) and choose **ToneGuard: Open Markdown Preview**.

## Ignore controls

Inline ignore:

```md
This sentence uses marketing language. <!-- dwg:ignore-line -->
```

Block ignore:

```md
<!-- dwg:ignore buzzword, puffery -->
This section is allowed to contain banned examples.
<!-- dwg:end-ignore -->
```

## CLI usage

If you want CI or batch runs:

```bash
cargo install dwg-cli --force
dwg-cli --config layth-style.yml --strict docs/
```

## Troubleshooting (Linux)

If you see `GLIBC_2.xx not found`, the bundled binaries were built on a newer distro than your machine.

- Install `dwg-cli` + `dwg-lsp` from source and set `dwg.cliCommand` / `dwg.command` to use PATH.
- Or rebuild the VSIX on Ubuntu 22.04 and reinstall it.

## Feedback

Please visit my GitHub for feedback:

- https://github.com/editnori/toneguard/issues
