# Release Checklist for Contributors

Whenever you cut a new ToneGuard release (tag and VSIX), make sure you also publish both Rust crates so the global `cargo install dwg-cli` flow keeps working.

1. Update versions in:
   - `core/Cargo.toml`
   - `cli/Cargo.toml` (remember to bump the `dwg-core` dependency version)
   - `Cargo.toml` / `Cargo.lock` / `layth-style.yml` / VSIX metadata as needed
2. Commit the changes and tag the release.
3. Publish crates:
   ```bash
   cargo publish -p dwg-core
   # wait ~60 seconds for the index to refresh
   cargo publish -p dwg-cli
   ```
4. Package and upload the VSIX to the marketplaces.

Skip step 3 only if the crate version already exists on crates.io. Otherwise the extension’s global install path will break.
