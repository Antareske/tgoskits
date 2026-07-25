#!/bin/sh
# board-server.sh — SG2002 iperf3 server lifecycle with /proc/net/dev snapshots.
#
# Deployed to the board at /tmp/board-server.sh.  The PC-side controller
# (board-controller.py) opens an SSH channel and runs:
#
#   sh /tmp/board-server.sh <port> <warmup_flag>
#
# Semantics (see net-bench-sg2002-board-final-plan.md §2):
#   1. Emit NET_STATS_BEGIN, cat /proc/net/dev, NET_STATS_END — before snapshot.
#   2. Emit SERVER_READY so the PC knows it is safe to start iperf3 -c.
#   3. iperf3 -s -1 — accept exactly one client, block until finished, then exit.
#   4. Emit NET_STATS_BEGIN, cat /proc/net/dev, NET_STATS_END — after snapshot.
#
# iperf3 -s -1 is the key synchronisation primitive: the server handles one
# client and exits, so the after-snapshot is gated by an OS-visible process exit
# rather than a sleep/poll guess.  Requires iperf3 >= 3.10.
#
# Written for busybox ash — no bash-isms.

PORT="${1:-5201}"
WARMUP="${2:-0}"

fail() {
    echo "BOARD_SERVER_ERROR: $*"
    exit 1
}

# ---- sync point 1: before snapshot ----------------------------------------

echo "NET_STATS_BEGIN warmup=${WARMUP}"
cat /proc/net/dev
echo "NET_STATS_END"

# ---- notify controller ----------------------------------------------------

echo "SERVER_READY"

# ---- sync point 2: serve one client, block until done ---------------------

iperf3 -s -1 -p "$PORT" >/dev/null || fail "iperf3 -s -1 exited with code $?"

# ---- sync point 3: after snapshot -----------------------------------------

echo "NET_STATS_BEGIN warmup=${WARMUP}"
cat /proc/net/dev
echo "NET_STATS_END"
