#!/usr/bin/env bash

set -euo pipefail

echo "Deploying API service..."
pnpm install --frozen-lockfile
pnpm run build
pnpm run deploy
