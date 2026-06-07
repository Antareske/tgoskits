#!/usr/bin/env bash
set -euo pipefail

SERIAL="${SERIAL:-/dev/ttyUSB0}"

while true; do
  printf '\003' > "$SERIAL"
  sleep 0.2
done
