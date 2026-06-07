#!/usr/bin/env bash
set -euo pipefail

SERIAL="${SERIAL:-/dev/ttyUSB0}"
exec cargo xtask starry quick-start orangepi-5-plus run --serial "$SERIAL"
