#!/bin/sh
# One round of the tool, driven end to end the way the demo is driven by hand.
#
# The demo's load is iperf3, run by the operator.  Board images ship iperf3;
# the QEMU root filesystem does not, so it has to be injected into the overlay
# (NET_EVENTS_IPERF3_DIR in prebuild.sh) for this check to use it.  When it is
# not there, or when the host has no iperf3 server listening, the harness's own
# HTTP server stands in — the point of the check is the attach → count → print
# chain, and which load ran is printed below rather than assumed.
#
# This lives in a script rather than in the case's `shell_cmd` because those go
# through the serial console, where a long multi-line block is a standing
# hazard.

set -u

# The QEMU user-mode gateway: the guest's route to whatever the host runs,
# `iperf3 -s` included.
PEER=${NET_EVENTS_PEER:-10.0.2.2}
PORT=${NET_EVENTS_PORT:-18392}
IPERF_SECONDS=${NET_EVENTS_IPERF_SECONDS:-3}
OUT=/tmp/net-events-check.log
REQUESTS=8
ATTEMPTS=30

fail() {
    echo "NET_EVENTS_FAILED: $1"
    exit 1
}

# iperf3 against the host, retried while the interface is still getting its
# address from DHCP.
load_iperf3() {
    attempt=1
    while [ "$attempt" -le "$ATTEMPTS" ]; do
        if iperf3 -c "$PEER" -t "$IPERF_SECONDS" >/tmp/net-events-iperf3.log 2>&1; then
            echo "  load: iperf3 -c $PEER -t $IPERF_SECONDS"
            grep -E "sender|receiver" /tmp/net-events-iperf3.log | tail -2 | sed 's/^/  /'
            return 0
        fi
        sleep 1
        attempt=$((attempt + 1))
    done
    return 1
}

# The harness's HTTP server, retried the same way.
load_download() {
    attempt=1
    while [ "$attempt" -le "$ATTEMPTS" ]; do
        sent=0
        request=0
        while [ "$request" -lt "$REQUESTS" ]; do
            if timeout 5 busybox wget -q -O /dev/null "http://$PEER:$PORT/" 2>/dev/null; then
                sent=$((sent + 1))
            fi
            request=$((request + 1))
        done
        if [ "$sent" -gt 0 ]; then
            echo "  load: HTTP download from $PEER:$PORT ($sent request(s))"
            return 0
        fi
        sleep 1
        attempt=$((attempt + 1))
    done
    return 1
}

/usr/bin/net-events >"$OUT" 2>&1 &
tool=$!

if command -v iperf3 >/dev/null 2>&1 && load_iperf3; then
    :
elif load_download; then
    :
else
    kill "$tool" 2>/dev/null
    fail "no load reached the peer at $PEER (neither iperf3 nor the HTTP server)"
fi

# Ask for a reading, then stop the tool; stopping prints one more.
kill -USR1 "$tool" 2>/dev/null || fail "the tool exited before the reading"
sleep 1
kill "$tool" 2>/dev/null
wait "$tool" 2>/dev/null

cat "$OUT"

# The traffic reaches the queue poll rounds and both frame events; anything
# else the run happens to drive is reported, not required.
awk '
    /^  queue_poll_round/ { if ($3 ~ /^\+[1-9]/) poll = 1 }
    /^  tx_submit/        { if ($3 ~ /^\+[1-9]/) tx = 1 }
    /^  rx_publish/       { if ($3 ~ /^\+[1-9]/) rx = 1 }
    END { exit (poll && tx && rx) ? 0 : 1 }
' "$OUT" || fail "a reading did not show the load (poll=$poll tx=$tx rx=$rx)"

echo "NET_EVENTS_PASSED"
