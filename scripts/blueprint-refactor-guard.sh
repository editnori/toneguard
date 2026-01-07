#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/blueprint-refactor-guard.sh [--base <rev>] [--config <path>] [--mapping <path>] [--out-dir <dir>] [--cli <path>] [--require]

What it does:
  - Builds a "before" blueprint from a git base revision (via worktree)
  - Builds an "after" blueprint from your current working tree
  - Writes a diff report + mapping template under reports/
  - Optionally fails if removed files are not mapped

Defaults:
  --base:   merge-base(HEAD, origin/main) (falls back to merge-base(HEAD, main))
  --config: layth-style.yml
  --mapping: flow-blueprint-mapping.yml
  --out-dir: reports
  --cli:    uses cargo run -p dwg-cli

Examples:
  scripts/blueprint-refactor-guard.sh
  scripts/blueprint-refactor-guard.sh --require
  scripts/blueprint-refactor-guard.sh --base v0.1.71 --require --mapping flows/blueprint-mapping.yml
EOF
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BASE_REV=""
CONFIG="layth-style.yml"
MAPPING="flow-blueprint-mapping.yml"
OUT_DIR="reports"
REQUIRE="0"
CLI_PATH=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base)
      BASE_REV="$2"
      shift 2
      ;;
    --config)
      CONFIG="$2"
      shift 2
      ;;
    --mapping)
      MAPPING="$2"
      shift 2
      ;;
    --out-dir)
      OUT_DIR="$2"
      shift 2
      ;;
    --cli)
      CLI_PATH="$2"
      shift 2
      ;;
    --require)
      REQUIRE="1"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown arg: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$BASE_REV" ]]; then
  if git rev-parse --verify origin/main >/dev/null 2>&1; then
    BASE_REV="$(git merge-base HEAD origin/main)"
  else
    BASE_REV="$(git merge-base HEAD main)"
  fi
fi

mkdir -p "$OUT_DIR"

TEMPLATE_PATH="${OUT_DIR%/}/flow-blueprint-mapping.template.yml"
DIFF_PATH="${OUT_DIR%/}/flow-blueprint-diff.base.json"

TMP="$(mktemp -d)"
BASE_TREE="${TMP}/base"
BEFORE_BP="${TMP}/flow-blueprint.before.json"
AFTER_BP="${TMP}/flow-blueprint.after.json"

cleanup() {
  git worktree remove --force "$BASE_TREE" >/dev/null 2>&1 || true
  rm -rf "$TMP"
}
trap cleanup EXIT

if [[ -n "$CLI_PATH" ]]; then
  CLI=("$CLI_PATH")
else
  CLI=(cargo run -p dwg-cli --quiet --)
fi

echo "Blueprint guard: base=$BASE_REV"

git worktree add --detach "$BASE_TREE" "$BASE_REV" >/dev/null

(
  cd "$BASE_TREE"
  "${CLI[@]}" flow blueprint --config "$CONFIG" --format json --out "$BEFORE_BP" .
)

"${CLI[@]}" flow blueprint --config "$CONFIG" --format json --out "$AFTER_BP" .

"${CLI[@]}" flow blueprint diff \
  --before "$BEFORE_BP" \
  --after "$AFTER_BP" \
  --out "$DIFF_PATH" \
  --write-mapping "$TEMPLATE_PATH" >/dev/null

echo "Wrote: $DIFF_PATH"
echo "Wrote: $TEMPLATE_PATH"

if [[ "$REQUIRE" == "1" ]]; then
  if [[ ! -f "$MAPPING" ]]; then
    echo "Missing mapping file: $MAPPING" >&2
    echo "Template: $TEMPLATE_PATH" >&2
    exit 2
  fi
  "${CLI[@]}" flow blueprint diff \
    --before "$BEFORE_BP" \
    --after "$AFTER_BP" \
    --require-mapping "$MAPPING" \
    --out "$DIFF_PATH" >/dev/null
  echo "Mapping check: OK ($MAPPING)"
fi
