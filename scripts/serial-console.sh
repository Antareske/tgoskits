#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VENV_PY="$SCRIPT_DIR/../.venv-serial-console/bin/python"

if [ ! -x "$VENV_PY" ]; then
  echo "missing venv: $VENV_PY" >&2
  echo "create it with: python3 -m venv .venv-serial-console" >&2
  exit 1
fi

exec "$VENV_PY" "$SCRIPT_DIR/serial-console.py" "$@"
