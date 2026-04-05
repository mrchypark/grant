#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:?usage: ./scripts/dev-empty-down.sh <root>}"

for name in certd trailbase; do
  pid_file="$ROOT/$name.pid"
  if [[ -f "$pid_file" ]]; then
    pid="$(cat "$pid_file")"
    if kill -0 "$pid" >/dev/null 2>&1; then
      kill "$pid"
    fi
  fi
done

echo "stopped stack rooted at $ROOT"
