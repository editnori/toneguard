# ToneGuard
ToneGuard scans Markdown and plain-text documentation for stylistic tics that commonly appear in large language model prose. It flags puffery, buzzwords, over-formal transitions, marketing clichés, negative parallelism, rule-of-three lists, connector overload, template conclusions, and other AI tells while protecting Layth’s direct, lowercase-forward voice.

## Components

- `core/`: Rust library with deterministic parsing heuristics and configurable rules.
- `cli/`: Command-line entry point for humans and CI pipelines.
- `vscode-extension/`: VS Code scaffold that shells out to the CLI for live diagnostics.
- `layth-style.yml`: Default configuration tuned to Layth’s writing patterns.

## Quick start

```bash
cargo run -p dwg-cli -- examples/sample.md
```

The CLI reports AI-style detections, category counts, and a density score (flags per 100 words). Use `--json` for machine-readable output consumed by the VS Code extension or CI pipelines.

## Configuration

Rules, thresholds, and whitelists live in `layth-style.yml`. The defaults include expanded buzzword throttles, dedicated transition throttles, and hard bans for marketing clichés and puffery. Files can locally disable checks via:

```markdown
<!-- dwg:off -->
verbatim text or quotes
<!-- dwg:on -->
```

Key sections of the YAML:

- `buzzwords.throttle`: verbs, adjectives, and jargon that should appear rarely.
- `transitions.throttle`: essay-style connectors (e.g. “furthermore”, “consequently”).
- `puffery.ban` / `marketing_cliches.ban`: phrases that nearly always oversell.
- `templates.ban`: regexes for boilerplate openers and negative parallelism.
- `weasel.ban`: vague attributions and hedged qualifiers.

## Extension

`vscode-extension/` contains a Node/TypeScript extension that spawns the CLI on save (debounced) and streams diagnostics into VS Code. Package it with `npm run package` after running `npm install` and building the Rust CLI.

## CI usage

```bash
cargo install --path cli --force
dwg lint docs/ --strict
```

`--strict` forces non-zero exit when the density exceeds the warning threshold, keeping pull requests honest.
