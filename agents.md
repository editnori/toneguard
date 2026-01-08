# Release checklist

Tooling rules for this repo:

- Rust: `cargo` only
- Extension + webviews: `bun` only (no npm/yarn)

This repo uses tags + a VSIX for local installs.
If you want `cargo install dwg-cli` to work for everyone, you also need to publish both crates.

1. Update versions in:
   - `core/Cargo.toml`
   - `cli/Cargo.toml` (remember to bump the `dwg-core` dependency version)
   - `vscode-extension/package.json` (VSIX version)
   - `Cargo.toml` / `Cargo.lock` / `layth-style.yml` as needed
2. Commit the changes and tag the release.
3. Publish crates:
   ```bash
   cargo publish -p dwg-core
   # wait ~60 seconds for the index to refresh
   cargo publish -p dwg-cli
   ```
4. Build and package the VSIX (JS uses Bun):
   ```bash
   cargo test --workspace
   ./scripts/install-local.sh
   ```
5. Linux compatibility sanity check (optional, but catches bad builds):
   - If you ship bundled Linux binaries, build them on Ubuntu 22.04 (GLIBC 2.35) or older.
   - CI enforces a GLIBC baseline for `dwg` + `dwg-lsp` via `readelf -V`.
6. (Optional) Run the blueprint refactor guard against the previous tag:
   ```bash
   ./scripts/blueprint-refactor-guard.sh --base <tag> --require
   ```
7. Upload the VSIX to the marketplaces.

Skip step 3 only if the crate version already exists on crates.io. Otherwise the extension's global install path will break.
