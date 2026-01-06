#!/bin/bash
# Cross-compile dwg-lsp and dwg CLI for all supported platforms
# Requires: cross (cargo install cross)
# Usage: ./scripts/build-binaries.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
LSP_DIR="$PROJECT_ROOT/lsp"
CLI_DIR="$PROJECT_ROOT/cli"
OUTPUT_DIR="$PROJECT_ROOT/vscode-extension/bin"

# Target platforms
declare -A TARGETS=(
    ["linux-x64"]="x86_64-unknown-linux-gnu"
    ["linux-arm64"]="aarch64-unknown-linux-gnu"
    ["darwin-x64"]="x86_64-apple-darwin"
    ["darwin-arm64"]="aarch64-apple-darwin"
    ["win32-x64"]="x86_64-pc-windows-gnu"
)

# Binary names per platform
get_lsp_binary_name() {
    local platform=$1
    if [[ "$platform" == win32-* ]]; then
        echo "dwg-lsp.exe"
    else
        echo "dwg-lsp"
    fi
}

get_cli_binary_name() {
    local platform=$1
    if [[ "$platform" == win32-* ]]; then
        echo "dwg.exe"
    else
        echo "dwg"
    fi
}

echo "=== ToneGuard LSP Cross-Compilation ==="
echo "Project root: $PROJECT_ROOT"
echo "Output dir: $OUTPUT_DIR"
echo ""

# Check if cross is installed
if ! command -v cross &> /dev/null; then
    echo "Installing cross..."
    cargo install cross --git https://github.com/cross-rs/cross
fi

# Build for each platform
for platform in "${!TARGETS[@]}"; do
    target="${TARGETS[$platform]}"
    lsp_binary_name=$(get_lsp_binary_name "$platform")
    cli_binary_name=$(get_cli_binary_name "$platform")
    lsp_output_path="$OUTPUT_DIR/$platform/$lsp_binary_name"
    cli_output_path="$OUTPUT_DIR/$platform/$cli_binary_name"
    
    echo "Building for $platform ($target)..."
    mkdir -p "$OUTPUT_DIR/$platform"
    
    # Use cross for cross-compilation
    cd "$PROJECT_ROOT"
    
    if [[ "$platform" == darwin-* ]]; then
        # macOS targets require special handling - use cargo-zigbuild if available
        if command -v cargo-zigbuild &> /dev/null; then
            echo "  Using cargo-zigbuild for macOS..."
            cargo zigbuild --release --target "$target" -p dwg-lsp
            cargo zigbuild --release --target "$target" -p dwg-cli
        else
            echo "  WARNING: macOS cross-compilation requires cargo-zigbuild or a macOS host"
            echo "  Skipping $platform - will be built in GitHub Actions on macOS runner"
            continue
        fi
    else
        # Linux and Windows can use cross
        cross build --release --target "$target" -p dwg-lsp
        cross build --release --target "$target" -p dwg-cli
    fi
    
    # Copy binary to output
    lsp_src_binary="$PROJECT_ROOT/target/$target/release/$lsp_binary_name"
    cli_src_binary="$PROJECT_ROOT/target/$target/release/dwg-cli$( [[ "$platform" == win32-* ]] && echo '.exe' )"
    if [ -f "$lsp_src_binary" ]; then
        cp "$lsp_src_binary" "$lsp_output_path"
        echo "  ✓ Copied to $lsp_output_path"
        
        # Strip the binary to reduce size (not for Windows)
        if [[ "$platform" != win32-* ]] && command -v strip &> /dev/null; then
            strip "$lsp_output_path" 2>/dev/null || true
            echo "  ✓ Stripped LSP binary"
        fi
    else
        echo "  ✗ LSP binary not found at $lsp_src_binary"
    fi

    if [ -f "$cli_src_binary" ]; then
        cp "$cli_src_binary" "$cli_output_path"
        echo "  ✓ Copied to $cli_output_path"
        if [[ "$platform" != win32-* ]] && command -v strip &> /dev/null; then
            strip "$cli_output_path" 2>/dev/null || true
            echo "  ✓ Stripped CLI binary"
        fi
    else
        echo "  ✗ CLI binary not found at $cli_src_binary"
    fi
    
    echo ""
done

# Show results
echo "=== Build Results ==="
for platform in "${!TARGETS[@]}"; do
    lsp_binary_name=$(get_lsp_binary_name "$platform")
    cli_binary_name=$(get_cli_binary_name "$platform")
    lsp_output_path="$OUTPUT_DIR/$platform/$lsp_binary_name"
    cli_output_path="$OUTPUT_DIR/$platform/$cli_binary_name"
    if [ -f "$lsp_output_path" ]; then
        size=$(du -h "$lsp_output_path" | cut -f1)
        echo "✓ $platform (lsp): $size"
    else
        echo "✗ $platform (lsp): not built"
    fi
    if [ -f "$cli_output_path" ]; then
        size=$(du -h "$cli_output_path" | cut -f1)
        echo "✓ $platform (cli): $size"
    else
        echo "✗ $platform (cli): not built"
    fi
done

echo ""
echo "Done! Binaries are in $OUTPUT_DIR"
