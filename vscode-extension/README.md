# Deterministic Writing Guard VS Code Extension

This extension shells out to the `dwg-cli` binary to highlight AI-style prose patterns in Markdown and plaintext documents. Diagnostics surface as warnings with inline suggestions sourced from the deterministic ruleset.

## Setup

1. Build or install the CLI:

```bash
cargo install --path cli --force
```

2. Install Node dependencies and compile the extension:

```bash
cd vscode-extension
npm install
npm run compile
```

3. Press `F5` in VS Code to launch the extension in Extension Development Host.

Configuration keys (`dwg.command`, `dwg.configPath`, `dwg.debounceMs`) can be changed under **Settings → Extensions → Deterministic Writing Guard**.
