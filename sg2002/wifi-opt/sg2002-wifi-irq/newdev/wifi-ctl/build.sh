#!/bin/sh
# Cross-compiles wifi-ctl for the SG2002 board (RISC-V 64, musl static).
# Output: wifi-ctl (statically linked, ready for injection into the rootfs).
set -e

cd "$(dirname "$0")"

CC="${CC:-/opt/riscv64-linux-musl-cross/bin/riscv64-linux-musl-gcc}"

"$CC" -static -O2 -Wall -Wextra -o wifi-ctl wifi-ctl.c
"$(dirname "$CC")/../bin/riscv64-linux-musl-strip" wifi-ctl 2>/dev/null || true

echo "built: $(pwd)/wifi-ctl"
