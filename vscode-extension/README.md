# ToneGuard VS Code Extension

ToneGuard flags common AI-style writing patterns in Markdown and text files. It also ships flow tooling (audit, blueprint, CFG) for code review.

## Quick start

1. Install the extension.
2. Open a folder.
3. Open the ToneGuard view (Activity Bar icon) and click **Run**.

## What you get

- Live diagnostics for supported file types (Problems panel + inline squiggles)
- Dashboard (Review, Findings, Organizer, Settings)
- Flow Map (blueprint graph + clusters, function index, CFG viewer)
- Optional skill installer (Cursor / Claude Code / Codex)

## Configuration

ToneGuard looks for a workspace config first (`layth-style.yml`, `.toneguard.yml`). If none exists, it uses the bundled defaults.

Useful settings:

- `dwg.configPath`: path to a custom config file
- `dwg.profile`: force a profile for all files
- `dwg.strict`: fail on warn-level density
- `dwg.noRepoChecks`: skip repo-wide checks
- `dwg.uiTheme`: theme for Dashboard + Flow Map

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

## Feedback

Please visit my GitHub for feedback:

- https://github.com/editnori/toneguard/issues
