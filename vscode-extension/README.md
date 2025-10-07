# ToneGuard VS Code extension

## Overview
The ToneGuard VS Code extension shells out to the `dwg-cli` binary and surfaces deterministic writing diagnostics inside the editor. It mirrors the CLI rules so Markdown and plain-text files receive the same feedback without leaving VS Code.

## Quickstart
1. Install the CLI with `cargo install --path cli --force` or point to an existing binary in your PATH.
2. Run `npm install` inside `vscode-extension/`.
3. Execute `npm run compile` to build the TypeScript sources.
4. Package with `npx @vscode/vsce package` or press `F5` to launch an Extension Development Host.

```bash
npm install
npm run compile
npx @vscode/vsce package
```

## Dependencies
Install the extension prerequisites before building. Matching versions keep results aligned with the CLI:
- Node.js 18+ and npm.
- Rust 1.75+ when you build the CLI locally.
- `@vscode/vsce` for packaging a VSIX.

## Configuration
Tweak behaviour from VS Code settings or `settings.json`:
- `dwg.command`: absolute path to the `dwg-cli` binary.
- `dwg.configPath`: repository-relative path to `layth-style.yml`.
- `dwg.debounceMs`: delay before re-running the CLI after edits.
- `dwg.noRepoChecks`: skip hygiene warnings when you only want document diagnostics.

Use the "ToneGuard: Lint Workspace" command to push diagnostics for every file without opening them individually.

## Running tests
The extension reuses the CLI test suite. Run `cargo test` at the workspace root, then execute `npm run compile` inside `vscode-extension/` to ensure the TypeScript build succeeds.

## License
This extension ships under the MIT License. See the repository `LICENSE` file for details.

## Contributing
Open an issue or pull request with focused changes. Please run `cargo fmt`, `cargo test`, and `npm run compile` before submitting.
