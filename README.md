# ToneGuard

ToneGuard keeps docs grounded in plain language and gives you artifacts for code review. It works as a CLI and a VS Code extension.

## Overview

ToneGuard scans Markdown/text for AI-style writing patterns and structural slop. It also ships flow tools (audit, proposal, blueprint, CFG) so you can review logic changes with concrete outputs.

What you get:

- Writing lint (categories + per-glob profiles)
- Repo checks (lockfiles, suspicious names, large JSON, stray temp files)
- Flow guardrails (flow specs, audits, proposal artifacts)
- Blueprint graph (repo-wide file dependency map)
- Blueprint diff (refactor guard: require a mapping for removed files)
- Function index + CFG output (JSON or Mermaid)
- Dashboard + Flow Map UI in VS Code

## Quickstart

Start by adding a config file to the repo root (`layth-style.yml` or `.toneguard.yml`). Then run the CLI or use the extension Dashboard to generate reports under `reports/`.

### Command line

Install the CLI and run it on a repo. Use `--json` when you want a report file, and `--strict` in CI.

```bash
cargo install dwg-cli --force
dwg-cli --config layth-style.yml --json . > reports/markdown-lint.json
```

You can scope runs with `--profile`, or toggle categories with `--only`, `--enable`, and `--disable`. If you only want document diagnostics (no repo checks), pass `--no-repo-checks`.

### Extension

Build a VSIX and install it locally. The extension bundles `dwg-lsp` and `dwg` for your OS/arch by default.

```bash
./scripts/install-local.sh
```

Open the ToneGuard view, click **Run**, then review the files written to `reports/`.

### Flow tools

Flow tools create review artifacts for code changes. They are separate from writing lint.

```bash
dwg-cli flow audit --config layth-style.yml --out reports/flow-audit.json .
dwg-cli flow propose --config layth-style.yml --out reports/flow-proposal.md .
dwg-cli flow blueprint --config layth-style.yml --out reports/flow-blueprint.json .
dwg-cli flow index --config layth-style.yml --out reports/flow-index.json .
```

To generate a CFG (JSON + optional Mermaid), run:

```bash
dwg-cli flow graph --file path/to/file.rs --fn my_fn --with-logic --include-mermaid --out reports/cfg.json
```

To use blueprint diff as a refactor guard:

```bash
dwg-cli flow blueprint --out reports/flow-blueprint.before.json .
dwg-cli flow blueprint --out reports/flow-blueprint.after.json .
dwg-cli flow blueprint diff --before reports/flow-blueprint.before.json --after reports/flow-blueprint.after.json --write-mapping reports/flow-blueprint-mapping.yml
dwg-cli flow blueprint diff --before reports/flow-blueprint.before.json --after reports/flow-blueprint.after.json --require-mapping reports/flow-blueprint-mapping.yml
```

### Organizer

Organizer helps keep one-off scripts/data/output from leaking into the repo root. It can also generate a cleanup prompt for Cursor/Claude/Codex.

```bash
dwg-cli organize --config layth-style.yml --json --out reports/organization-report.json .
```

To generate a prompt instead of a report, pass `--prompt-for cursor` (or `claude`, `codex`).

## Dependencies

The Rust workspace uses Cargo. The VS Code extension build uses Bun.

- Rust 1.75+ (workspace build and tests)
- Bun (extension build), plus `@vscode/vsce` for packaging

## Configuration

`layth-style.yml` controls what gets scanned and what is ignored. The same config is used by the CLI, the LSP server, and the extension reports.

Key sections:

- `file_types`: which file types are linted
- `repo_rules.ignore_globs`: ignore paths (including `reports/**` to avoid lint loops)
- `profiles`: per-glob tuning (README vs docs vs notes)
- `flow_rules`: flow spec settings and audit ignore globs
- `organize_rules`: what counts as data/scripts/legacy files

This repo ignores `docs/**` and `examples/**` by default because they contain intentional bad examples. In a normal repo you probably want to remove those ignore globs.

## Running tests

Run the Rust tests from the repo root. For extension changes, run Bun lint and compile.

```bash
cargo fmt
cargo test --workspace
cd vscode-extension && bun install && bun run lint && bun run compile
```

## License

MIT. See `LICENSE` for the full text.

## Contributing

Open an issue or submit a focused PR. For feedback, use GitHub Issues: https://github.com/editnori/toneguard/issues
