#!/bin/bash
# Build CLI + LSP for the current platform, bundle into the VS Code extension,
# and optionally install the VSIX if the `code` command is available.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
OUTPUT_DIR="$PROJECT_ROOT/vscode-extension/bin"

platform_dir() {
    local platform
    local arch
    platform=$(uname -s)
    arch=$(uname -m)

    if [[ "$platform" == "Darwin" ]]; then
        if [[ "$arch" == "arm64" ]]; then
            echo "darwin-arm64"
        else
            echo "darwin-x64"
        fi
    elif [[ "$platform" == "Linux" ]]; then
        if [[ "$arch" == "aarch64" ]]; then
            echo "linux-arm64"
        else
            echo "linux-x64"
        fi
    elif [[ "$platform" == MINGW* || "$platform" == MSYS* || "$platform" == CYGWIN* ]]; then
        echo "win32-x64"
    else
        echo "${platform}-${arch}"
    fi
}

PLATFORM_DIR=$(platform_dir)
BIN_DIR="$OUTPUT_DIR/$PLATFORM_DIR"

cd "$PROJECT_ROOT"

echo "=== Building ToneGuard CLI + LSP ==="
cargo build --release -p dwg-lsp -p dwg-cli

mkdir -p "$BIN_DIR"

LSP_BIN="dwg-lsp"
CLI_BIN="dwg"
if [[ "$PLATFORM_DIR" == win32-* ]]; then
    LSP_BIN="dwg-lsp.exe"
    CLI_BIN="dwg.exe"
fi

if [[ -f "target/release/dwg-lsp.exe" ]]; then
    cp "target/release/dwg-lsp.exe" "$BIN_DIR/$LSP_BIN"
else
    cp "target/release/dwg-lsp" "$BIN_DIR/$LSP_BIN"
fi

if [[ -f "target/release/dwg-cli.exe" ]]; then
    cp "target/release/dwg-cli.exe" "$BIN_DIR/$CLI_BIN"
else
    cp "target/release/dwg-cli" "$BIN_DIR/$CLI_BIN"
fi

if command -v strip &> /dev/null && [[ "$PLATFORM_DIR" != win32-* ]]; then
    strip "$BIN_DIR/$LSP_BIN" 2>/dev/null || true
    strip "$BIN_DIR/$CLI_BIN" 2>/dev/null || true
fi

echo "Bundled binaries in $BIN_DIR"

if command -v bun &> /dev/null; then
    echo "=== Building VS Code extension ==="
    cd "$PROJECT_ROOT/vscode-extension"
    bun install
    bun run compile
    bunx @vscode/vsce package
    VSIX=$(ls -t *.vsix | head -n 1)
    echo "Built VSIX: $VSIX"

    if command -v code &> /dev/null; then
        echo "Installing VSIX via code..."
        code --install-extension "$VSIX" --force
        echo "VSIX installed."
    else
        echo "VSIX built. Install manually via VS Code: Extensions → Install from VSIX..."
    fi
else
    echo "bun not found. Skipping VS Code extension build."
fi
