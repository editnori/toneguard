# ToneGuard VS Code extension

## Overview
The ToneGuard VS Code extension connects to the `dwg-lsp` language server and surfaces deterministic writing diagnostics inside the editor. It keeps the analyzer hot in memory, updates diagnostics on edits, and provides quick fixes like `<!-- dwg:ignore-line -->`.

## Quickstart
1. Install the language server globally (once per machine):
   `cargo install dwg-lsp --force`
2. Grab the latest `toneguard-*.vsix` from the releases page, or build it yourself:

   ```bash
   git clone https://github.com/editnori/toneguard.git
   cd toneguard/vscode-extension
   bun install
   bun run compile
   bunx @vscode/vsce package
   ```

3. Open VS Code → **Extensions → Install from VSIX…** and choose the downloaded (or freshly built) package.

After step 1 the server binary lives in `%USERPROFILE%\.cargo\bin\dwg-lsp.exe`, which is already on VS Code's PATH. Unless you relocate it, leave `dwg.command` at the default (`dwg-lsp`).

### Useful CLI commands
The extension uses `dwg-lsp` for in-editor diagnostics. You can still script the CLI alongside editor usage:

- `dwg-cli --config layth-style.yml --strict .` – full workspace lint with repo hygiene.
- `dwg-cli --json --config layth-style.yml README.md` – machine-readable output for a single file.
- `dwg-cli comments src/ --config layth-style.yml` – comment hygiene report (add `--strip` to delete eligible comments).
- `dwg-cli --profile readme --only structure,marketing README.md` – force a profile and limit categories.
- `dwg-cli --no-repo-checks docs/` – skip repo hygiene when you only want document diagnostics.

## Dependencies
Install the extension prerequisites before building:
- [Bun](https://bun.sh) (recommended) or Node.js 18+.
- Rust 1.75+ when you build the CLI locally.
- `@vscode/vsce` for packaging a VSIX.

## Configuration
Tweak behaviour from VS Code settings or `settings.json`:
- `dwg.command`: absolute path to the `dwg-lsp` server binary.
- `dwg.configPath`: repository-relative path to `layth-style.yml`.
- `dwg.profile`: force a profile for all files (overrides glob matching).
- `dwg.onlyCategories` / `dwg.enableCategories` / `dwg.disableCategories`: filter which categories are surfaced.

Use the "ToneGuard: Lint Workspace" command to refresh diagnostics for open files.

## Running tests
The extension reuses the CLI test suite. Run `cargo test` at the workspace root, then execute `bun run compile` inside `vscode-extension/` to ensure the TypeScript build succeeds.

## License
This extension ships under the MIT License. See the repository `LICENSE` file for details.

## Contributing
Open an issue or pull request with focused changes. Please run `cargo fmt`, `cargo test`, and `bun run compile` before submitting.
