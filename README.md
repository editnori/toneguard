# ToneGuard
ToneGuard scans Markdown and plain-text documentation for stylistic tics that commonly appear in large language model prose. It flags puffery, buzzwords, over-formal transitions, marketing clichés, negative parallelism, rule-of-three lists, connector overload, template conclusions, and other AI tells while protecting Layth’s direct, lowercase-forward voice. The latest pass also enforces document structure (required headings per doc type), blocks rhetorical/question headings, limits bullet sprawl, and surfaces repo-level slop such as duplicate lockfiles, suspicious filename variants, and giant JSON dumps.

## Components

- `core/`: Rust library with deterministic parsing heuristics and configurable rules.
- `cli/`: Command-line entry point for humans and CI pipelines.
- `vscode-extension/`: VS Code scaffold that shells out to the CLI for live diagnostics.
- `layth-style.yml`: Default configuration tuned to Layth’s writing patterns.

## Quick start

```bash
cargo run -p dwg-cli -- examples/sample.md
```

The CLI reports AI-style detections, category counts, and a density score (flags per 100 words). Use `--json` for machine-readable output (now including `repo_issues`) consumed by the VS Code extension or CI pipelines. Repository slop warnings (duplicate lockfiles, banned directories, suspicious filenames, oversized JSON/YAML) are printed up front.

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
- `profile_defaults`: baseline structural limits (sentence length, cadence, duplicate sentences, CTA phrases, broad terms, question leads, exclamation density).
- `profiles`: per-glob overrides (required sections, heading caps, custom template phrases, term bans) for READMEs, tickets, RFCs, ADRs, postmortems, changelogs, and API docs.
- `repo_rules`: repo-wide hygiene (ignore globs, slop glob patterns, banned directories, suspicious filename regexes, large JSON/YAML caps, duplicate lockfile detection).
- `comment_policy`: repo-wide comment hygiene (max ratios, ignore/allow globs).

## Repo hygiene

`repo_rules` drive deterministic slop checks before per-file linting. By default ToneGuard warns when it spots:

- banned directories/files like `__pycache__/`, `.DS_Store`, `.idea/`, `Thumbs.db`
- suspicious filenames (`*_copy`, `*_final`, `cleanup_script.py`, etc.)
- both `package-lock.json` and `yarn.lock` in the same package
- JSON/YAML blobs over 500KB outside fixtures/data folders

Customise glob allow/ban lists in `layth-style.yml` to match your repos. Repo warnings appear in both human output and the JSON `repo_issues` array.

## CLI flags

Examples:

```bash
dwg --only structure,marketing --profile readme --set profile_defaults.min_sentences_per_section=2 README.md
dwg --disable transition,buzzword --no-repo-checks docs/
dwg --only-repo stray-markdown,dup-variants -- json
```

- `--profile <name>` force a profile for all files
- `--only/--enable/--disable <cat[,cat]>` filter categories
- `--set key=value` apply overrides (e.g., `profile_defaults.min_code_blocks=1`)
- `--no-repo-checks`, `--only-repo/--enable-repo/--disable-repo`

## Extension

`vscode-extension/` contains a Node/TypeScript extension that spawns the CLI on save (debounced) and streams diagnostics into VS Code. Install deps with `npm install`, build the CLI, run `npm run compile`, then package with `npx @vscode/vsce package`.

## CI usage

```bash
cargo install --path cli --force
dwg lint docs/ --strict
```

`--strict` forces non-zero exit when the density exceeds the warning threshold, keeping pull requests honest.

## Comment hygiene

ToneGuard can audit and optionally strip code comments:

```bash
dwg comments src/ --config layth-style.yml
dwg comments --strip            # remove full-line comments where allowed
```

Use `comment_policy` in the YAML to tune ratios and ignore/allow globs before running. Keywords now include `DIRTY` / “quick and dirty” markers, and `dwg comments --strip` respects ticket links when deciding whether to delete TODO/FIXME lines.
