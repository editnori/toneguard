# ToneGuard
ToneGuard keeps team docs grounded in plain language. It spots the marketing fluff and rigid transitions that slip into Markdown or text files after heavy editing.

## Overview
ToneGuard scans Markdown and plain-text documentation for hallmarks of AI-authored prose. It flags puffery, buzzwords, stiff transitions, and template conclusions while keeping Layth's direct voice intact.

The workspace ships a Rust analysis core, a CLI for local or CI runs, and a VS Code extension that reuses the CLI for live diagnostics.

## Components
ToneGuard is organised as a Cargo workspace with the following packages:

- `core/`: deterministic parser and rule engine.
- `cli/`: dwg CLI wrapper for local runs and CI pipelines.
- `vscode-extension/`: VS Code extension that shells out to the CLI.
- `layth-style.yml`: default rule set tuned for this repository.

Each crate shares a single version and configuration so updates stay consistent.

## Quickstart
Copy'paste friendly steps for getting ToneGuard running everywhere:

1. Install the CLI globally (run from any directory):

   ```bash
   cargo install dwg-cli --force
   dwg-cli --version
   ```

2. Lint any repository (no need to clone ToneGuard):

   ```bash
   cd path\to\your\project
   dwg-cli --config layth-style.yml --strict .
   ```

   If the repo doesnt already have a config, copy `layth-style.yml` from this project or point `--config` at a shared copy.

3. Install the VS Code extension once:

   - Download the latest `toneguard-*.vsix` from the releases page (or build it yourself, see **Extension**).
   - Open VS Code ' **Extensions ' Install from VSIX** and pick the file.
   - The extension shells out to the globally installed `dwg-cli`, so every workspace is ready to lint.

Keep a copy of `layth-style.yml` in each repo (or set `dwg.configPath` to a shared location) so both the CLI and the extension load the rules you expect.

## CLI command reference
Common invocations you can drop into any repository:

- Full lint with repo hygiene:\
  `dwg-cli --config layth-style.yml --strict .`
- Quiet JSON output (pipe into jq or CI tooling):\
  `dwg-cli --config layth-style.yml --json --strict .`
- Focus on a single profile or path:\
  `dwg-cli --profile readme README.md`
- Enable / disable categories on the fly:\
  `dwg-cli --only structure,marketing docs/README.md`\
  `dwg-cli --disable transition,buzzword notes/*.md`
- Skip repo checks when you only care about document lint:\
  `dwg-cli --no-repo-checks ./docs`
- Comment hygiene mode (report):\
  `dwg-cli comments src/ --config layth-style.yml`
- Comment hygiene mode (strip eligible comments):\
  `dwg-cli comments src/ --config layth-style.yml --strip`

## Dependencies
Install the following tools before building. They match the workspace's tested toolchain:
- Rust 1.75+ for the workspace.
- Node.js 18+ and npm for the VS Code extension build.
- `@vscode/vsce` when packaging a VSIX.

## Configuration
Override behaviour through `layth-style.yml`. Each section targets a specific area.

### Style safeguards
These keys throttle phrasing that sounds inflated. Adjust them when a document needs to allow a specific term:
- `buzzwords.throttle`: throttles marketing-heavy verbs and adjectives.
- `transitions.throttle`: limits formal connectors such as "however".
- `puffery.ban` and `marketing_cliches.ban`: block stock promotional copy.

### Structural controls
These settings enforce shape and hygiene:
- `profile_defaults`: sets baseline limits for sentence length and cadence checks.
- `profiles`: applies per-glob overrides for README files and other templates such as RFCs or tickets.
- `repo_rules`: runs a hygiene sweep covering ignore globs, duplicate lockfiles, suspicious filenames, and oversized JSON or YAML.
- `comment_policy`: enforces optional ratios for TODO and FIXME comments with allow and ignore globs.

Wrap verbatim text with the dwg comment fence (HTML comments named `dwg:off` and `dwg:on`) when a section must bypass the lint.

## Repo hygiene
Repo checks run before document linting and surface deterministic issues:

- banned directories or files such as `__pycache__/`, `.idea/`, or `.DS_Store`
- duplicate lockfiles (`package-lock.json` next to `yarn.lock`)
- oversized structured files outside fixture folders
- slop paths that resemble `copy`, `final`, or similar variants

Adjust the ignore and allow lists in `layth-style.yml` if your project needs exemptions.

## CLI flags
Common combinations:

```bash
dwg --profile readme --only structure,marketing README.md
dwg --disable transition,buzzword docs/
dwg --no-repo-checks --only-repo stray-markdown,dup-variants .
```

Additional options include:
- `--set key=value` for ad-hoc overrides such as `profile_defaults.min_sentences_per_section=2`.
- `--enable` and `--disable` to toggle categories without editing the config.
- Repo-level flags help focus the hygiene sweep. Use `--only-repo` to restrict categories and `--disable-repo` when you want a quiet pass.

## Running tests
Use the standard workspace commands:

```bash
cargo fmt
cargo test
npm install --prefix vscode-extension
npm run compile --prefix vscode-extension
```

## Extension
Build the VS Code extension after installing dependencies:

```bash
npm install --prefix vscode-extension
npm run compile --prefix vscode-extension
npx @vscode/vsce package --prefix vscode-extension
```

Install the resulting `toneguard-<version>.vsix` via "Extensions: Install from VSIX...".

## CI usage

```bash
cargo install dwg-cli --force
dwg --strict docs/
```

Use `--strict` in CI to enforce non-zero exits when densities cross the warning threshold.

## Comment hygiene
ToneGuard can audit and strip code comments when needed:

```bash
dwg comments src/ --config layth-style.yml
dwg comments --strip
```

Tune ratios and allow lists through `comment_policy` before running destructive operations.

## License
ToneGuard is licensed under the MIT License. See `LICENSE` for full terms. The MIT terms allow commercial and open-source redistribution so long as you include the original copyright and license text.

## Contributing
Open an issue or submit a pull request with a focused change set. Run the linters and tests before sending patches.




