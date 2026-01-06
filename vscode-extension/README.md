# ToneGuard VS Code Extension

**Zero-setup AI slop detection for Markdown.** Highlights generic, AI-style writing patterns instantly.

## Features

- **Zero Configuration Required** — Works immediately after install
- **Bundled Language Server** — Pre-compiled binaries for Windows, macOS, and Linux
- **Smart Defaults** — Built-in detection rules tuned for common AI writing patterns
- **Quick Fixes** — One-click ignore for false positives
- **Real-time Feedback** — Diagnostics update as you type
- **Sidebar Overview** — Run audits and browse findings from a dedicated ToneGuard panel

## Quick Start

1. **Install from Marketplace** — Search "ToneGuard" in VS Code Extensions
2. **Open any Markdown file** — Diagnostics appear automatically
3. **That's it!** — No configuration needed

## What It Detects

ToneGuard identifies common AI writing patterns:

| Category | Examples |
|----------|----------|
| **Buzzwords** | "leverage", "robust", "seamless", "cutting-edge" |
| **Puffery** | "world-class", "industry-leading", "best-in-class" |
| **Templates** | "In this article, we will...", "Let's dive in..." |
| **Weasel Words** | "some experts say", "studies have shown" |
| **Marketing Clichés** | "game-changer", "paradigm shift", "synergy" |
| **Formatting Slop** | Emoji bullets, excessive bold, mid-sentence questions |

## Configuration (Optional)

ToneGuard works out of the box, but you can customize it:

### Workspace Config

Add a `layth-style.yml` to your project root for custom rules:

```yaml
# Example: Disable transition warnings
limits:
  transitions_per_section: 10
  
# Add project-specific whitelisted terms
whitelist:
  buzzwords:
    - "kubernetes"  # Technical term, not slop
```

### VS Code Settings

| Setting | Description |
|---------|-------------|
| `dwg.configPath` | Path to custom config (defaults to workspace `layth-style.yml`) |
| `dwg.cliCommand` | Path to ToneGuard CLI (`dwg`) for flow audits |
| `dwg.profile` | Force a specific profile for all files |
| `dwg.disableCategories` | Hide specific diagnostic categories |
| `dwg.enableCategories` | Show additional categories |

## Commands

- **ToneGuard: Run Recommended Review** — Runs flow audit + generates a proposal (best first step)
- **ToneGuard: Lint Workspace** — Refresh diagnostics for all open files
- **ToneGuard: Flow Audit Workspace** — Run flow audit detectors and write a report
- **ToneGuard: Generate Flow Proposal (Markdown)** — Create a reviewable Markdown artifact from flow checks + audit
- **ToneGuard: New Flow Spec** — Scaffold a new flow spec file under `flows/`
- **ToneGuard: Show Server Info** — Display active server and config paths
- **ToneGuard: Install Logic Flow Guardrails Skill** — Install the flow guardrail prompt

## Ignoring False Positives

### Inline Ignore

```markdown
This sentence uses robust technology. <!-- dwg:ignore-line -->
```

### Block Ignore

```markdown
<!-- dwg:ignore buzzword, puffery -->
This marketing content won't be flagged for buzzwords or puffery.
<!-- dwg:end-ignore -->
```

## CLI Usage

For CI/CD or batch processing, install the CLI:

```bash
cargo install dwg-cli
dwg-cli --config layth-style.yml --strict docs/
```

## Requirements

- VS Code 1.86.0 or later
- Windows (x64), macOS (Intel/Apple Silicon), or Linux (x64/arm64)

## Troubleshooting

| Issue | Solution |
|-------|----------|
| No diagnostics | Run "ToneGuard: Show Server Info" to check status |
| Server not found | Extension includes bundled binary — reinstall if missing |
| Custom config not loading | Check `dwg.configPath` setting and file existence |

## License

MIT License. See [LICENSE](https://github.com/editnori/toneguard/blob/main/LICENSE).

## Links

- [GitHub Repository](https://github.com/editnori/toneguard)
- [Issue Tracker](https://github.com/editnori/toneguard/issues)
- [Full Documentation](https://github.com/editnori/toneguard#readme)
