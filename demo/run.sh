#!/usr/bin/env bash
# Builds the server and the client from this checkout and opens a demo folder in a VS Code window that runs them.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
demo="${1:-rails_app}"

cd "$repo"
bun run build:server
bun run build
exec code --new-window --extensionDevelopmentPath="$repo" "$repo/demo/$demo"
